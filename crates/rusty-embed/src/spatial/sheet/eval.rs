//! An expression, evaluated — every operation on a vector, a quaternion or a
//! matrix leaving its step behind as it goes.

use std::collections::{HashMap, HashSet};
use std::f64::consts::{PI, TAU};

use super::Live;
use super::parse::{Expr, Op};
use super::steps::{self, Step};
use super::value::{Kind, Note, Problem, Value};
use crate::spatial::{Euler, Frame, Quat, Vec2, Vec3, check};

/// Every function the sheet knows, by name: what a row may not be called.
pub const FUNCTIONS: &[&str] = &[
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sqrt",
    "abs",
    "deg",
    "rad",
    "quat",
    "axis_angle",
    "euler",
    "to_euler",
    "dcm",
    "from_dcm",
    "mat",
    "transpose",
    "det",
    "conj",
    "inv",
    "norm",
    "normalize",
    "dot",
    "cross",
    "project",
    "angle",
    "axis",
    "rotate",
    "to_world",
    "to_body",
    "slerp",
    "from_to",
    "delta",
    "to_rotvec",
    "from_rotvec",
    "integrate",
    "integrate_linear",
    "accel_at_rest",
    "gravity_body",
    "tilt",
    "check",
    "tel",
    "truth",
    "truth_rate",
];

/// A name the sheet gives a value without a row: `pi`, the axes, level.
fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "pi" | "π" => Value::Number(PI),
        "X" => Value::Vec3(Vec3::X),
        "Y" => Value::Vec3(Vec3::Y),
        "Z" => Value::Vec3(Vec3::Z),
        "identity" => Value::Quat(Quat::IDENTITY),
        _ => return None,
    })
}

/// What an expression can see: the rows above it, which names come later,
/// the frame and the live values.
pub struct Scope<'a> {
    /// The value of every named row above, or the fact that it has none.
    pub rows: &'a HashMap<String, Option<Value>>,
    /// Names given to rows below.
    pub later: &'a HashSet<String>,
    pub frame: Frame,
    pub live: &'a Live,
}

/// One expression's evaluation: its steps and notes as it went.
pub struct Eval<'a> {
    scope: &'a Scope<'a>,
    pub steps: Vec<Step>,
    pub notes: Vec<Note>,
    /// It read the firmware's telemetry or the plant.
    pub live: bool,
}

type Answer = Result<Value, Problem>;

impl<'a> Eval<'a> {
    pub fn new(scope: &'a Scope<'a>) -> Self {
        Eval {
            scope,
            steps: Vec::new(),
            notes: Vec::new(),
            live: false,
        }
    }

    pub fn expr(&mut self, expr: &Expr) -> Answer {
        match expr {
            Expr::Number(v) => Ok(Value::Number(*v)),
            Expr::Text(_) => Err(Problem::StrayText),
            Expr::Name(name) => self.name(name),
            Expr::Call { name, args } => self.call(name, args),
            Expr::Tuple(items) => self.tuple(items),
            Expr::Neg(inner) => {
                let v = self.expr(inner)?;
                self.negate(v)
            }
            Expr::Binary { op, left, right } => {
                let l = self.expr(left)?;
                let r = self.expr(right)?;
                self.binary(*op, l, r)
            }
            Expr::Field { of, field } => {
                let v = self.expr(of)?;
                field_of(v, field)
            }
            Expr::Degrees(inner) => match self.expr(inner)? {
                Value::Number(v) => Ok(Value::Angle(v.to_radians())),
                other => Err(Problem::Operands {
                    op: '°',
                    left: other.kind(),
                    right: Kind::Number,
                }),
            },
            Expr::Radians(inner) => match self.expr(inner)? {
                Value::Number(v) | Value::Angle(v) => Ok(Value::Angle(v)),
                other => Err(Problem::Operands {
                    op: 'r',
                    left: other.kind(),
                    right: Kind::Number,
                }),
            },
        }
    }

    fn name(&mut self, name: &str) -> Answer {
        if let Some(value) = self.scope.rows.get(name) {
            return value.clone().ok_or(Problem::Broken { name: name.into() });
        }
        if let Some(value) = constant(name) {
            return Ok(value);
        }
        if self.scope.later.contains(name) {
            return Err(Problem::Later { name: name.into() });
        }
        if FUNCTIONS.contains(&name) {
            return Err(Problem::NotCallable { name: name.into() });
        }
        Err(Problem::Unknown { name: name.into() })
    }

    fn tuple(&mut self, items: &[Expr]) -> Answer {
        let mut numbers = Vec::with_capacity(items.len());
        for item in items {
            match self.expr(item)? {
                Value::Number(v) | Value::Angle(v) => numbers.push(v),
                _ => return Err(Problem::Tuple { len: items.len() }),
            }
        }
        match numbers.as_slice() {
            [x, y] => Ok(Value::Vec2(Vec2::new(*x, *y))),
            [x, y, z] => Ok(Value::Vec3(Vec3::new(*x, *y, *z))),
            [_, _, _, _] => Err(Problem::FourTuple),
            _ => Err(Problem::Tuple { len: items.len() }),
        }
    }

    fn negate(&mut self, v: Value) -> Answer {
        Ok(match v {
            Value::Number(x) => Value::Number(-x),
            Value::Angle(x) => Value::Angle(-x),
            Value::Vec2(v) => Value::Vec2(-v),
            Value::Vec3(v) => Value::Vec3(-v),
            Value::Quat(q) => Value::Quat(-q),
            Value::Mat3(m) => Value::Mat3(m * -1.0),
            Value::Euler(e) => Value::Euler(Euler::new(-e.roll, -e.pitch, -e.yaw)),
            other => {
                return Err(Problem::Operands {
                    op: '-',
                    left: other.kind(),
                    right: other.kind(),
                });
            }
        })
    }

    fn binary(&mut self, op: Op, l: Value, r: Value) -> Answer {
        use Value::*;
        let mismatch = |l: &Value, r: &Value| Problem::Operands {
            op: op.symbol(),
            left: l.kind(),
            right: r.kind(),
        };
        match op {
            Op::Add | Op::Sub => {
                let plus = op == Op::Add;
                let sign = if plus { 1.0 } else { -1.0 };
                Ok(match (&l, &r) {
                    (Number(a), Number(b)) => Number(a + sign * b),
                    (Angle(a) | Number(a), Angle(b) | Number(b)) => Angle(a + sign * b),
                    (Vec2(a), Vec2(b)) => {
                        let (a, b) = (steps::lift(*a), steps::lift(*b));
                        self.steps.push(if plus {
                            steps::add(a, b, true)
                        } else {
                            steps::sub(a, b, true)
                        });
                        let c = a + b * sign;
                        Vec2(crate::spatial::Vec2::new(c.x, c.y))
                    }
                    (Vec3(a), Vec3(b)) => {
                        self.steps.push(if plus {
                            steps::add(*a, *b, false)
                        } else {
                            steps::sub(*a, *b, false)
                        });
                        Vec3(*a + *b * sign)
                    }
                    (Quat(a), Quat(b)) => Quat(*a + *b * sign),
                    (Mat3(a), Mat3(b)) => Mat3(*a + *b * sign),
                    _ => return Err(mismatch(&l, &r)),
                })
            }
            Op::Mul => self.multiply(l, r),
            Op::Div => {
                let Some(k) = r.scalar() else {
                    return Err(mismatch(&l, &r));
                };
                if k == 0.0 {
                    return Err(Problem::DivideByZero);
                }
                Ok(match (&l, &r) {
                    (Angle(a), Angle(_)) => Number(a / k),
                    (Angle(a), Number(_)) => Angle(a / k),
                    (Number(a), _) => Number(a / k),
                    (Vec2(v), _) => Vec2(*v / k),
                    (Vec3(v), _) => Vec3(*v / k),
                    (Quat(q), _) => Quat(*q * (1.0 / k)),
                    (Mat3(m), _) => Mat3(*m * (1.0 / k)),
                    _ => return Err(mismatch(&l, &r)),
                })
            }
            Op::Pow => match (l.scalar(), r.scalar()) {
                (Some(a), Some(b)) => Ok(Number(a.powf(b))),
                _ => Err(mismatch(&l, &r)),
            },
        }
    }

    fn multiply(&mut self, l: Value, r: Value) -> Answer {
        use Value::*;
        Ok(match (&l, &r) {
            (Number(a), Number(b)) => Number(a * b),
            (Angle(a), Number(b)) | (Number(b), Angle(a)) => Angle(a * b),
            (Angle(a), Angle(b)) => Number(a * b),
            (s @ (Number(_) | Angle(_)), v) | (v, s @ (Number(_) | Angle(_))) => {
                let k = s.scalar().unwrap_or(1.0);
                match v {
                    Vec2(v) => {
                        self.steps.push(steps::scale(k, steps::lift(*v), true));
                        Vec2(*v * k)
                    }
                    Vec3(v) => {
                        self.steps.push(steps::scale(k, *v, false));
                        Vec3(*v * k)
                    }
                    Quat(q) => Quat(*q * k),
                    Mat3(m) => Mat3(*m * k),
                    _ => {
                        return Err(Problem::Operands {
                            op: '*',
                            left: l.kind(),
                            right: r.kind(),
                        });
                    }
                }
            }
            (Quat(a), Quat(b)) => {
                let ab = *a * *b;
                self.steps.push(steps::compose(*a, *b, ab));
                self.steps.push(steps::compose_second(*a, ab));
                Quat(ab)
            }
            // `q * v` turns v, as nalgebra and glam read it.
            (Quat(q), Vec3(v)) => Vec3(self.turn(*q, *v, false)),
            (Mat3(m), Vec3(v)) => {
                let out = m.mul_vec(*v);
                self.steps.push(steps::mat_vec(*m, *v, out));
                Vec3(out)
            }
            (Mat3(a), Mat3(b)) => {
                let out = *a * *b;
                self.steps.push(steps::mat_mat(out));
                Mat3(out)
            }
            _ => {
                return Err(Problem::Operands {
                    op: '*',
                    left: l.kind(),
                    right: r.kind(),
                });
            }
        })
    }

    /// `v` turned by `q` (or, `to_body`, by `q*`), with its working.
    fn turn(&mut self, q: Quat, v: Vec3, to_body: bool) -> Vec3 {
        self.unit(q);
        let turned = if to_body {
            q.conj().rotate(v)
        } else {
            q.rotate(v)
        };
        self.steps.extend(steps::sandwich(q, v, turned, to_body));
        turned
    }

    /// Say so when a quaternion used as a rotation is not one.
    fn unit(&mut self, q: Quat) {
        let norm = q.norm();
        if (norm - 1.0).abs() > 1e-6 {
            let note = Note::NotUnit { norm };
            if !self.notes.contains(&note) {
                self.notes.push(note);
            }
        }
    }

    fn call(&mut self, name: &str, args: &[Expr]) -> Answer {
        // `tel` takes a name, not a value.
        if name == "tel" {
            let [Expr::Text(channel)] = args else {
                return Err(Problem::Arguments {
                    function: name.into(),
                    got: args.iter().map(|_| Kind::Number).collect(),
                });
            };
            self.live = true;
            return self
                .scope
                .live
                .channels
                .get(channel)
                .map(|v| Value::Number(*v))
                .ok_or_else(|| Problem::NoChannel {
                    name: channel.clone(),
                });
        }
        if !FUNCTIONS.contains(&name) {
            if self.scope.rows.contains_key(name) || constant(name).is_some() {
                return Err(Problem::NotCallable { name: name.into() });
            }
            return Err(Problem::Unknown { name: name.into() });
        }
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(self.expr(arg)?);
        }
        // A plain number more than a turn, where an angle is taken, was
        // probably meant in degrees: said, and taken as radians.
        let bare = |i: usize| {
            matches!(args.get(i), Some(Expr::Number(v)) if v.abs() > TAU)
                || matches!(args.get(i), Some(Expr::Neg(inner)) if matches!(**inner, Expr::Number(v) if v > TAU))
        };
        let result = self.function(name, &values, &bare)?;
        Ok(result)
    }

    fn angle_arg(&mut self, value: f64, bare: bool) -> f64 {
        if bare {
            let note = Note::BareAngle { value };
            if !self.notes.contains(&note) {
                self.notes.push(note);
            }
        }
        value
    }

    fn function(&mut self, name: &str, args: &[Value], bare: &dyn Fn(usize) -> bool) -> Answer {
        use Value::*;
        let wrong = || Problem::Arguments {
            function: name.into(),
            got: args.iter().map(Value::kind).collect(),
        };
        let scalar = |v: &Value| v.scalar();
        Ok(match (name, args) {
            // ---- plain arithmetic, no working to show ----
            ("sin", [a]) => Number(scalar(a).ok_or_else(wrong)?.sin()),
            ("cos", [a]) => Number(scalar(a).ok_or_else(wrong)?.cos()),
            ("tan", [a]) => Number(scalar(a).ok_or_else(wrong)?.tan()),
            ("asin", [Number(a)]) => Angle(a.clamp(-1.0, 1.0).asin()),
            ("acos", [Number(a)]) => Angle(a.clamp(-1.0, 1.0).acos()),
            ("atan", [Number(a)]) => Angle(a.atan()),
            ("atan2", [y, x]) => Angle(
                scalar(y)
                    .ok_or_else(wrong)?
                    .atan2(scalar(x).ok_or_else(wrong)?),
            ),
            ("sqrt", [Number(a)]) => Number(a.sqrt()),
            ("abs", [Number(a)]) => Number(a.abs()),
            ("abs", [Angle(a)]) => Angle(a.abs()),
            // An angle in degrees, as a plain number to read or print.
            ("deg", [a]) => Number(scalar(a).ok_or_else(wrong)?.to_degrees()),
            // A number of degrees, as an angle.
            ("rad", [Number(a)]) => Angle(a.to_radians()),

            // ---- building rotations ----
            ("quat", [w, x, y, z]) => {
                let q = crate::spatial::Quat::new(
                    scalar(w).ok_or_else(wrong)?,
                    scalar(x).ok_or_else(wrong)?,
                    scalar(y).ok_or_else(wrong)?,
                    scalar(z).ok_or_else(wrong)?,
                );
                self.steps.push(steps::quaternion(q));
                Quat(q)
            }
            ("axis_angle", [Vec3(axis), a]) => {
                let angle = self.angle_arg(scalar(a).ok_or_else(wrong)?, bare(1));
                let q = crate::spatial::Quat::from_axis_angle(*axis, angle)
                    .ok_or(Problem::NoDirection)?;
                self.steps.push(steps::axis_angle(*axis, angle, q));
                Quat(q)
            }
            ("euler", [r, p, y]) => {
                let e = crate::spatial::Euler::new(
                    self.angle_arg(scalar(r).ok_or_else(wrong)?, bare(0)),
                    self.angle_arg(scalar(p).ok_or_else(wrong)?, bare(1)),
                    self.angle_arg(scalar(y).ok_or_else(wrong)?, bare(2)),
                );
                self.steps.extend(steps::euler_turns(e));
                Quat(crate::spatial::Quat::from_euler(e))
            }
            ("to_euler", [Quat(q)]) => {
                self.unit(*q);
                let e = q.to_euler();
                if e.at_pole() {
                    self.notes.push(Note::Pole);
                }
                self.steps.push(steps::to_euler(*q, e));
                Euler(e)
            }
            ("dcm", [Quat(q)]) => {
                self.unit(*q);
                let m = q.to_mat3();
                self.steps.push(steps::matrix(*q, m));
                Mat3(m)
            }
            ("from_dcm", [Mat3(m)]) => {
                let q = crate::spatial::Quat::from_mat3(*m);
                self.steps.push(steps::from_matrix(*m, q));
                Quat(q)
            }
            ("mat", [Vec3(r0), Vec3(r1), Vec3(r2)]) => {
                Mat3(crate::spatial::Mat3::from_rows(*r0, *r1, *r2))
            }
            ("transpose", [Mat3(m)]) => {
                let t = m.transpose();
                self.steps.push(steps::transpose(*m, t));
                Mat3(t)
            }
            ("det", [Mat3(m)]) => {
                let d = m.det();
                self.steps.push(steps::det(*m, d));
                Number(d)
            }
            ("conj", [Quat(q)]) => {
                let c = q.conj();
                self.steps.push(steps::conjugate(*q, c, false));
                Quat(c)
            }
            ("inv", [Quat(q)]) => {
                let c = q.inverse().ok_or(Problem::DivideByZero)?;
                self.steps.push(steps::conjugate(*q, c, true));
                Quat(c)
            }

            // ---- vectors ----
            ("norm", [Vec2(v)]) => {
                self.steps.push(steps::norm(steps::lift(*v), true));
                Number(v.norm())
            }
            ("norm", [Vec3(v)]) => {
                self.steps.push(steps::norm(*v, false));
                Number(v.norm())
            }
            ("norm", [Quat(q)]) => Number(q.norm()),
            ("normalize", [Vec2(v)]) => {
                let n = v.normalized().ok_or(Problem::NoDirection)?;
                self.steps
                    .push(steps::normalize(steps::lift(*v), steps::lift(n), true));
                Vec2(n)
            }
            ("normalize", [Vec3(v)]) => {
                let n = v.normalized().ok_or(Problem::NoDirection)?;
                self.steps.push(steps::normalize(*v, n, false));
                Vec3(n)
            }
            ("normalize", [Quat(q)]) => {
                let n = q.normalized().ok_or(Problem::NoDirection)?;
                self.steps.push(steps::normalize_quat(*q, n));
                Quat(n)
            }
            ("dot", [Vec2(a), Vec2(b)]) => {
                self.steps
                    .push(steps::dot(steps::lift(*a), steps::lift(*b), true));
                Number(a.dot(*b))
            }
            ("dot", [Vec3(a), Vec3(b)]) => {
                self.steps.push(steps::dot(*a, *b, false));
                Number(a.dot(*b))
            }
            ("dot", [Quat(a), Quat(b)]) => Number(a.dot(*b)),
            ("cross", [Vec3(a), Vec3(b)]) => {
                self.steps.push(steps::cross(*a, *b));
                Vec3(a.cross(*b))
            }
            ("cross", [Vec2(a), Vec2(b)]) => {
                self.steps.push(steps::cross2(*a, *b));
                Number(a.cross(*b))
            }
            ("project", [Vec3(a), Vec3(b)]) => {
                let along = a.project_onto(*b).ok_or(Problem::NoDirection)?;
                self.steps.push(steps::project(*a, *b, along, false));
                Vec3(along)
            }
            ("project", [Vec2(a), Vec2(b)]) => {
                let along = a.project_onto(*b).ok_or(Problem::NoDirection)?;
                self.steps.push(steps::project(
                    steps::lift(*a),
                    steps::lift(*b),
                    steps::lift(along),
                    true,
                ));
                Vec2(along)
            }
            ("angle", [Vec2(a), Vec2(b)]) => {
                let theta = a.angle_to(*b).ok_or(Problem::NoDirection)?;
                self.steps.push(steps::angle_between(
                    steps::lift(*a),
                    steps::lift(*b),
                    theta,
                    true,
                ));
                Angle(theta)
            }
            ("angle", [Vec3(a), Vec3(b)]) => {
                let theta = a.angle_to(*b).ok_or(Problem::NoDirection)?;
                self.steps.push(steps::angle_between(*a, *b, theta, false));
                Angle(theta)
            }
            ("angle", [Quat(a), Quat(b)]) => {
                let theta = a.angle_to(*b);
                self.steps.push(steps::attitude_error(*a, *b, theta));
                Angle(theta)
            }
            ("angle", [Quat(q)]) => {
                self.steps.push(steps::quaternion(*q));
                Angle(q.axis_angle().map_or(0.0, |(_, angle)| angle))
            }
            ("axis", [Quat(q)]) => {
                self.steps.push(steps::quaternion(*q));
                Vec3(q.axis_angle().ok_or(Problem::NoDirection)?.0)
            }

            // ---- turning things ----
            ("rotate", [Quat(q), Vec3(v)]) | ("to_world", [Quat(q), Vec3(v)]) => {
                Vec3(self.turn(*q, *v, false))
            }
            ("to_body", [Quat(q), Vec3(v)]) => Vec3(self.turn(*q, *v, true)),
            ("rotate", [a, Vec2(v)]) if a.scalar().is_some() => {
                let angle = self.angle_arg(scalar(a).ok_or_else(wrong)?, bare(0));
                let turned = v.rotated(angle);
                self.steps.push(steps::turn2(angle, *v, turned));
                Vec2(turned)
            }
            ("slerp", [Quat(a), Quat(b), t]) => {
                let t = scalar(t).ok_or_else(wrong)?;
                let q = a.slerp(*b, t);
                self.steps.push(steps::slerp(*a, *b, t, q));
                Quat(q)
            }
            ("from_to", [Vec3(a), Vec3(b)]) => {
                let q = crate::spatial::Quat::from_to(*a, *b).ok_or(Problem::NoDirection)?;
                self.steps.push(steps::from_to(*a, *b, q));
                Quat(q)
            }
            ("delta", [Quat(a), Quat(b)]) => {
                let d = a.delta_to(*b);
                self.steps.push(steps::delta(*a, *b, d));
                Quat(d)
            }
            ("to_rotvec", [Quat(q)]) => {
                let v = q.to_rotvec();
                self.steps.push(steps::to_rotvec(*q, v));
                Vec3(v)
            }
            ("from_rotvec", [Vec3(v)]) => {
                let q = crate::spatial::Quat::from_rotvec(*v);
                self.steps.push(steps::from_rotvec(*v, q));
                Quat(q)
            }
            ("integrate", [Quat(q), Vec3(w), dt]) => {
                let dt = scalar(dt).ok_or_else(wrong)?;
                self.unit(*q);
                let next = q.integrate(*w, dt);
                self.steps.push(steps::integrate(*q, *w, dt, next));
                Quat(next)
            }
            ("integrate_linear", [Quat(q), Vec3(w), dt]) => {
                let dt = scalar(dt).ok_or_else(wrong)?;
                let next = q.integrate_linear(*w, dt);
                self.steps.push(steps::integrate_linear(*q, *w, dt, next));
                Quat(next)
            }

            // ---- gravity ----
            ("accel_at_rest", [Quat(q)]) => {
                self.unit(*q);
                let reading = self.scope.frame.at_rest(*q, 1.0);
                self.steps
                    .push(steps::at_rest(*q, self.scope.frame.up(), reading, false));
                Vec3(reading)
            }
            ("gravity_body", [Quat(q)]) => {
                self.unit(*q);
                let reading = -self.scope.frame.at_rest(*q, 1.0);
                self.steps
                    .push(steps::at_rest(*q, self.scope.frame.up(), reading, true));
                Vec3(reading)
            }
            ("tilt", [Vec3(acc)]) => {
                let e = self.scope.frame.tilt(*acc).ok_or(Problem::NoDirection)?;
                self.steps
                    .push(steps::tilt(*acc, self.scope.frame.up().z, e));
                Euler(e)
            }

            // ---- two answers that should agree ----
            ("check", [mine, reference]) => {
                let (verdict, working) = match (mine, reference) {
                    (Quat(a), Quat(b)) => {
                        let v = check::quaternions(*a, *b);
                        (v, steps::check_rotation(*a, *b, v.error))
                    }
                    (Euler(a), Euler(b)) => {
                        let v = check::eulers(*a, *b);
                        let (qa, qb) = (
                            crate::spatial::Quat::from_euler(*a),
                            crate::spatial::Quat::from_euler(*b),
                        );
                        (v, steps::check_rotation(qa, qb, v.error))
                    }
                    (Vec3(a), Vec3(b)) => (check::vectors(*a, *b), steps::check_vector(*a, *b)),
                    (Angle(a), Angle(b) | Number(b)) | (Number(a), Angle(b)) => {
                        (check::angles(*a, *b), steps::check_number(*a, *b, true))
                    }
                    (Number(a), Number(b)) => {
                        (check::numbers(*a, *b), steps::check_number(*a, *b, false))
                    }
                    _ => return Err(wrong()),
                };
                self.steps.push(working);
                Verdict(verdict)
            }

            // ---- the simulator ----
            ("truth", []) => {
                self.live = true;
                Quat(self.scope.live.truth.ok_or(Problem::NoPlant)?)
            }
            ("truth_rate", []) => {
                self.live = true;
                Vec3(self.scope.live.truth_rate.ok_or(Problem::NoPlant)?)
            }
            _ => return Err(wrong()),
        })
    }
}

fn field_of(v: Value, field: &str) -> Answer {
    use Value::*;
    let missing = |v: &Value| Problem::NoField {
        kind: v.kind(),
        field: field.into(),
    };
    Ok(match (&v, field) {
        (Vec2(p), "x") => Number(p.x),
        (Vec2(p), "y") => Number(p.y),
        (Vec3(p), "x") => Number(p.x),
        (Vec3(p), "y") => Number(p.y),
        (Vec3(p), "z") => Number(p.z),
        (Quat(q), "w") => Number(q.w),
        (Quat(q), "x") => Number(q.x),
        (Quat(q), "y") => Number(q.y),
        (Quat(q), "z") => Number(q.z),
        (Euler(e), "roll") => Angle(e.roll),
        (Euler(e), "pitch") => Angle(e.pitch),
        (Euler(e), "yaw") => Angle(e.yaw),
        (Verdict(v), "error") => Number(v.error),
        _ => return Err(missing(&v)),
    })
}
