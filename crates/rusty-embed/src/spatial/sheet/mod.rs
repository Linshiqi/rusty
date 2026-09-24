//! The math toolbox's sheet: rows of `name = expression`, read top to
//! bottom, each worked out with its steps showing.
//!
//! A worksheet rather than a form of fields, because testing attitude code
//! is a chain — build an attitude from three angles, turn gravity into the
//! body with it, read the tilt back, check it against what the firmware
//! printed — and each link wants to be a name the next can use. A row
//! whose whole expression is one number gets a slider, so an angle can be
//! swept and everything below it follows.
//!
//! The language is small and says what it will not do: a vector times a
//! vector is refused with `dot` and `cross` named, four numbers in brackets
//! are refused because `(w, x, y, z)` and `(x, y, z, w)` are both somebody's
//! convention, and a plain number more than a turn where an angle goes is
//! taken as radians *and said*, since it was probably degrees.

mod eval;
mod lex;
mod parse;
pub mod steps;
mod value;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

pub use eval::FUNCTIONS;
pub use steps::{Remark, Role, Shape, Step, What};
pub use value::{Kind, Note, Problem, Value};

use super::{Frame, Quat, Vec3};
use eval::{Eval, Scope};
use parse::{Expr, Line};

/// A sheet as it is kept: the frame it is written in, and its rows as
/// typed.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MathSheet {
    #[serde(default)]
    pub frame: Frame,
    #[serde(default)]
    pub rows: Vec<String>,
}

/// What a running simulation offers a sheet: the firmware's newest value of
/// every telemetry channel, and the plant's own attitude and rates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    pub channels: HashMap<String, f64>,
    pub truth: Option<Quat>,
    pub truth_rate: Option<Vec3>,
}

/// One row, read and worked out.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: Option<String>,
    /// `None` for a row with nothing on it but a remark.
    pub value: Option<Result<Value, Problem>>,
    pub steps: Vec<Step>,
    pub notes: Vec<Note>,
    /// It reads the firmware or the plant, directly or through a row above.
    pub live: bool,
    pub slider: Option<Slider>,
}

/// A row that is one number, offered as a slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slider {
    pub unit: Unit,
    /// In the unit it was written in: degrees for `30°`.
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Plain,
    Degrees,
    Radians,
}

/// Every row of a sheet, in order, each seeing only the rows above it.
pub fn evaluate(sheet: &MathSheet, live: &Live) -> Vec<Row> {
    let lines: Vec<Result<Line, Problem>> = sheet.rows.iter().map(|r| parse::line(r)).collect();
    let names: Vec<Option<String>> = lines
        .iter()
        .zip(&sheet.rows)
        .map(|(line, text)| match line {
            Ok(Line::Named { name, .. }) => Some(name.clone()),
            // A row that does not read still says what it meant to be
            // called, so a row using it says *it* is broken rather than that
            // the name means nothing.
            Err(_) => leading_name(text),
            Ok(_) => None,
        })
        .collect();

    let mut known: HashMap<String, Option<Value>> = HashMap::new();
    let mut live_rows: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(lines.len());
    for (i, line) in lines.into_iter().enumerate() {
        let later: HashSet<String> = names[i + 1..].iter().flatten().cloned().collect();
        let blank = Row {
            name: names[i].clone(),
            value: None,
            steps: Vec::new(),
            notes: Vec::new(),
            live: false,
            slider: None,
        };
        let row = match line {
            Err(problem) => Row {
                value: Some(Err(problem)),
                ..blank
            },
            Ok(Line::Blank) => blank,
            Ok(Line::Bare(expr)) => {
                work(&expr, blank, &known, &live_rows, &later, sheet.frame, live)
            }
            Ok(Line::Named { name, expr }) => {
                if FUNCTIONS.contains(&name.as_str()) {
                    Row {
                        value: Some(Err(Problem::Reserved { name })),
                        ..blank
                    }
                } else if known.contains_key(&name) {
                    // The first keeps the name; this one says why it cannot.
                    out.push(Row {
                        value: Some(Err(Problem::Twice { name })),
                        name: None,
                        ..blank
                    });
                    continue;
                } else {
                    work(&expr, blank, &known, &live_rows, &later, sheet.frame, live)
                }
            }
        };
        if let Some(name) = &row.name
            && !FUNCTIONS.contains(&name.as_str())
        {
            let value = row.value.clone().and_then(Result::ok);
            known.insert(name.clone(), value);
            if row.live {
                live_rows.insert(name.clone());
            }
        }
        out.push(row);
    }
    out
}

fn work(
    expr: &Expr,
    row: Row,
    known: &HashMap<String, Option<Value>>,
    live_rows: &HashSet<String>,
    later: &HashSet<String>,
    frame: Frame,
    live: &Live,
) -> Row {
    let scope = Scope {
        rows: known,
        later,
        frame,
        live,
    };
    let mut eval = Eval::new(&scope);
    let value = eval.expr(expr).and_then(|v| {
        if v.is_finite() {
            Ok(v)
        } else {
            Err(Problem::NotFinite)
        }
    });
    let reads_live = eval.live || names_in(expr).iter().any(|n| live_rows.contains(*n));
    Row {
        value: Some(value),
        steps: eval.steps,
        notes: eval.notes,
        live: reads_live,
        slider: slider_of(expr),
        ..row
    }
}

/// Every name an expression reads.
fn names_in(expr: &Expr) -> Vec<&str> {
    let mut out = Vec::new();
    fn walk<'e>(e: &'e Expr, out: &mut Vec<&'e str>) {
        match e {
            Expr::Name(n) => out.push(n),
            Expr::Call { args, .. } | Expr::Tuple(args) => args.iter().for_each(|a| walk(a, out)),
            Expr::Neg(inner) | Expr::Degrees(inner) | Expr::Radians(inner) => walk(inner, out),
            Expr::Field { of, .. } => walk(of, out),
            Expr::Binary { left, right, .. } => {
                walk(left, out);
                walk(right, out);
            }
            Expr::Number(_) | Expr::Text(_) => {}
        }
    }
    walk(expr, &mut out);
    out
}

/// `name` from `name = …`, read without the rest of the row.
fn leading_name(text: &str) -> Option<String> {
    let (left, _) = text.split_once('=')?;
    let left = left.trim();
    (!left.is_empty() && left.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then(|| left.to_string())
}

/// A slider for a row that is one number: an angle in degrees swept round
/// a turn, one in radians the same, or a plain fraction from 0 to 1 — a `t`
/// for `slerp`. Other plain numbers get none: a `dt` of 0.002 on a slider
/// from 0 to 1 is a slider that can only say 0.
fn slider_of(expr: &Expr) -> Option<Slider> {
    let (sign, inner) = match expr {
        Expr::Neg(inner) => (-1.0, &**inner),
        other => (1.0, other),
    };
    let number = |e: &Expr| match e {
        Expr::Number(v) => Some(*v),
        Expr::Neg(inner) => match **inner {
            Expr::Number(v) => Some(-v),
            _ => None,
        },
        _ => None,
    };
    match inner {
        Expr::Number(v) => {
            let v = sign * v;
            let hundredths = (v * 100.0).round() / 100.0;
            ((0.0..=1.0).contains(&v) && (v - hundredths).abs() < 1e-12).then_some(Slider {
                unit: Unit::Plain,
                value: v,
                min: 0.0,
                max: 1.0,
                step: 0.01,
            })
        }
        Expr::Degrees(n) => number(n).map(|v| {
            let v = sign * v;
            Slider {
                unit: Unit::Degrees,
                value: v,
                min: v.min(-180.0),
                max: v.max(180.0),
                step: 1.0,
            }
        }),
        Expr::Radians(n) => number(n).map(|v| {
            let v = sign * v;
            let pi = std::f64::consts::PI;
            Slider {
                unit: Unit::Radians,
                value: v,
                min: v.min(-pi),
                max: v.max(pi),
                step: 0.01,
            }
        }),
        _ => None,
    }
}

/// The row with its one number set to `value`, in the unit it was written
/// in, its name and any remark kept; `None` for a row that is not one
/// number.
pub fn set_literal(text: &str, value: f64) -> Option<String> {
    let (name, expr) = match parse::line(text).ok()? {
        Line::Named { name, expr } => (Some(name), expr),
        Line::Bare(expr) => (None, expr),
        Line::Blank => return None,
    };
    let slider = slider_of(&expr)?;
    let trimmed = |v: f64, places: usize| {
        let text = format!("{v:.places$}");
        let text = if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            text
        };
        if text == "-0" { "0".to_string() } else { text }
    };
    let number = match slider.unit {
        Unit::Degrees => format!("{}°", trimmed(value, 2)),
        Unit::Radians => format!("{} rad", trimmed(value, 4)),
        Unit::Plain => trimmed(value, 4),
    };
    let remark = text
        .find('#')
        .map(|i| format!(" {}", text[i..].trim_end()))
        .unwrap_or_default();
    Some(match name {
        Some(name) => format!("{name} = {number}{remark}"),
        None => format!("{number}{remark}"),
    })
}

/// A sheet to start from, named for the panel's menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Example {
    pub id: &'static str,
    pub frame: Frame,
    pub rows: &'static [&'static str],
}

impl Example {
    pub fn sheet(&self) -> MathSheet {
        MathSheet {
            frame: self.frame,
            rows: self.rows.iter().map(|r| r.to_string()).collect(),
        }
    }
}

/// The attitude arithmetic a flight controller is made of, one sheet each.
pub const EXAMPLES: &[Example] = &[
    Example {
        id: "euler",
        frame: Frame::ZUp,
        rows: &[
            "roll = 30°",
            "pitch = 15°",
            "yaw = 60°",
            "q = euler(roll, pitch, yaw)",
            "angles = to_euler(q)",
            "R = dcm(q)",
        ],
    },
    Example {
        id: "frames",
        frame: Frame::ZUp,
        rows: &[
            "q = euler(20°, -10°, 45°)",
            "nose = to_world(q, X)",
            "up = to_body(q, Z)",
            "back = to_world(q, up)",
        ],
    },
    Example {
        id: "gravity",
        frame: Frame::ZUp,
        rows: &[
            "roll = 25°",
            "pitch = -10°",
            "q = euler(roll, pitch, 0°)",
            "acc = accel_at_rest(q)",
            "tilted = tilt(acc)",
            "check(tilted, to_euler(q))",
        ],
    },
    Example {
        id: "gyro",
        frame: Frame::ZUp,
        rows: &[
            "q0 = euler(10°, 5°, 30°)",
            "w = (0.5, -0.2, 1.0)",
            "dt = 0.002",
            "q1 = integrate(q0, w, dt)",
            "q1_linear = integrate_linear(q0, w, dt)",
            "drift = angle(q1, q1_linear)",
        ],
    },
    Example {
        id: "compose",
        frame: Frame::ZUp,
        rows: &[
            "a = axis_angle(Z, 90°)",
            "b = axis_angle(X, 90°)",
            "ab = a * b",
            "ba = b * a",
            "apart = angle(ab, ba)",
        ],
    },
    Example {
        id: "error",
        frame: Frame::ZUp,
        rows: &[
            "target = euler(0°, 10°, 90°)",
            "now = euler(5°, 2°, 80°)",
            "err = angle(now, target)",
            "turn = delta(now, target)",
            "e = to_rotvec(turn)",
        ],
    },
    Example {
        id: "slerp",
        frame: Frame::ZUp,
        rows: &[
            "a = euler(0°, 0°, 0°)",
            "b = euler(60°, 30°, 120°)",
            "t = 0.5",
            "q = slerp(a, b, t)",
        ],
    },
    Example {
        id: "check",
        frame: Frame::ZUp,
        rows: &[
            "reference = euler(20°, -15°, 60°)",
            "mine = quat(0.2134, -0.0252, 0.5078, 0.8342)",
            "check(mine, reference)",
        ],
    },
    Example {
        id: "live",
        frame: Frame::ZUp,
        rows: &[
            "estimate = euler(tel(\"roll\"), tel(\"pitch\"), tel(\"yaw\"))",
            "rates = (tel(\"gx\"), tel(\"gy\"), tel(\"gz\"))",
            "plant = truth()",
            "err = angle(estimate, plant)",
        ],
    },
    Example {
        id: "rigid",
        frame: Frame::ZUp,
        rows: &[
            "q = axis_angle(Z, 90°)",
            "t = (1, 0, 0)",
            "p = (1, 0.5, 0)",
            "turn_then_move = rotate(q, p) + t",
            "move_then_turn = rotate(q, p + t)",
        ],
    },
    Example {
        id: "plane",
        frame: Frame::ZUp,
        rows: &[
            "a = (2, 1)",
            "b = (0.5, 2)",
            "sum = a + b",
            "along = dot(a, b)",
            "area = cross(a, b)",
            "turned = rotate(30°, a)",
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::check::Relation;

    fn sheet(rows: &[&str]) -> MathSheet {
        MathSheet {
            frame: Frame::ZUp,
            rows: rows.iter().map(|r| r.to_string()).collect(),
        }
    }

    fn values(rows: &[&str]) -> Vec<Option<Result<Value, Problem>>> {
        evaluate(&sheet(rows), &Live::default())
            .into_iter()
            .map(|r| r.value)
            .collect()
    }

    fn value(rows: &[&str]) -> Result<Value, Problem> {
        values(rows)
            .pop()
            .flatten()
            .expect("the last row has a value")
    }

    #[test]
    fn rows_read_top_to_bottom_and_name_what_they_cannot_see() {
        let v = values(&["a = 2", "b = a * 3", "c = d", "d = 1", "e = nope"]);
        assert_eq!(v[1], Some(Ok(Value::Number(6.0))));
        assert_eq!(v[2], Some(Err(Problem::Later { name: "d".into() })));
        assert_eq!(
            v[4],
            Some(Err(Problem::Unknown {
                name: "nope".into()
            }))
        );
    }

    #[test]
    fn a_name_is_given_once_and_never_to_a_function() {
        let v = values(&["a = 1", "a = 2", "euler = 3", "b = a"]);
        assert_eq!(v[1], Some(Err(Problem::Twice { name: "a".into() })));
        assert_eq!(
            v[2],
            Some(Err(Problem::Reserved {
                name: "euler".into()
            }))
        );
        // The first `a` kept its name.
        assert_eq!(v[3], Some(Ok(Value::Number(1.0))));
    }

    /// A row that is broken, or does not even read, breaks the rows that use
    /// it by name — not as an unknown name.
    #[test]
    fn a_broken_row_is_named_by_the_rows_that_use_it() {
        let v = values(&[
            "a = (1, 2",
            "b = a + 1",
            "c = normalize((0, 0, 0))",
            "d = c",
        ]);
        assert!(matches!(v[0], Some(Err(Problem::Unclosed { .. }))));
        assert_eq!(v[1], Some(Err(Problem::Broken { name: "a".into() })));
        assert_eq!(v[2], Some(Err(Problem::NoDirection)));
        assert_eq!(v[3], Some(Err(Problem::Broken { name: "c".into() })));
    }

    #[test]
    fn the_language_refuses_what_it_cannot_decide() {
        assert_eq!(
            value(&["(1, 2, 3) * (4, 5, 6)"]),
            Err(Problem::Operands {
                op: '*',
                left: Kind::Vec3,
                right: Kind::Vec3
            })
        );
        assert_eq!(value(&["(1, 0, 0, 0)"]), Err(Problem::FourTuple));
        assert_eq!(value(&["(1, 2) / 0"]), Err(Problem::DivideByZero));
        assert_eq!(value(&["sqrt(-1)"]), Err(Problem::NotFinite));
        assert!(matches!(
            value(&["euler((1, 2, 3))"]),
            Err(Problem::Arguments { .. })
        ));
        assert_eq!(value(&["\"roll\""]), Err(Problem::StrayText));
        assert_eq!(
            value(&["q = quat(1, 0, 0, 0)", "q.roll"]),
            Err(Problem::NoField {
                kind: Kind::Quat,
                field: "roll".into()
            })
        );
    }

    #[test]
    fn angles_keep_their_unit_and_a_suspicious_one_is_said() {
        assert_eq!(value(&["30°"]), Ok(Value::Angle(30f64.to_radians())));
        assert_eq!(value(&["deg(pi / 2)"]), Ok(Value::Number(90.0)));
        assert_eq!(value(&["30° + 15°"]), Ok(Value::Angle(45f64.to_radians())));
        assert_eq!(value(&["90° / 45°"]), Ok(Value::Number(2.0)));
        let rows = evaluate(&sheet(&["euler(30, 0, 0)"]), &Live::default());
        assert_eq!(rows[0].notes, vec![Note::BareAngle { value: 30.0 }]);
        let rows = evaluate(&sheet(&["euler(0.5, 0, 0)"]), &Live::default());
        assert!(rows[0].notes.is_empty());
    }

    #[test]
    fn a_quaternion_turns_a_vector_and_shows_how() {
        let rows = evaluate(
            &sheet(&["q = axis_angle(Z, 90°)", "v = rotate(q, X)", "w = q * X"]),
            &Live::default(),
        );
        let Some(Ok(Value::Vec3(v))) = rows[1].value else {
            panic!("{:?}", rows[1].value);
        };
        assert!((v - Vec3::Y).norm() < 1e-12);
        assert_eq!(rows[2].value, rows[1].value);
        let whats: Vec<What> = rows[1].steps.iter().map(|s| s.what).collect();
        assert_eq!(whats, vec![What::ToWorld, What::Rodrigues]);
    }

    #[test]
    fn an_attitude_off_the_unit_sphere_is_said_when_it_turns_something() {
        let rows = evaluate(&sheet(&["rotate(quat(2, 0, 0, 0), X)"]), &Live::default());
        assert_eq!(rows[0].notes, vec![Note::NotUnit { norm: 2.0 }]);
        assert_eq!(
            rows[0].value,
            Some(Ok(Value::Vec3(Vec3::new(4.0, 0.0, 0.0))))
        );
    }

    #[test]
    fn euler_angles_show_their_three_turns_and_their_pole() {
        let rows = evaluate(
            &sheet(&["q = euler(10°, 90°, 30°)", "to_euler(q)"]),
            &Live::default(),
        );
        let whats: Vec<What> = rows[0].steps.iter().map(|s| s.what).collect();
        assert_eq!(
            whats,
            vec![What::EulerYaw, What::EulerPitch, What::EulerRoll]
        );
        assert_eq!(rows[1].notes, vec![Note::Pole]);
        assert_eq!(rows[1].steps[0].what, What::ToEulerPole);
    }

    #[test]
    fn a_row_reading_the_firmware_is_live_and_so_is_everything_using_it() {
        let mut live = Live::default();
        live.channels.insert("roll".into(), 0.25);
        let rows = evaluate(
            &sheet(&[
                "r = tel(\"roll\")",
                "q = euler(r, 0, 0)",
                "k = 2",
                "p = truth()",
            ]),
            &live,
        );
        assert_eq!(rows[0].value, Some(Ok(Value::Number(0.25))));
        assert!(rows[0].live && rows[1].live && !rows[2].live);
        assert_eq!(rows[3].value, Some(Err(Problem::NoPlant)));
        let rows = evaluate(&sheet(&["tel(\"pitch\")"]), &live);
        assert_eq!(
            rows[0].value,
            Some(Err(Problem::NoChannel {
                name: "pitch".into()
            }))
        );
    }

    #[test]
    fn one_number_is_a_slider_and_setting_it_keeps_the_row() {
        let rows = evaluate(
            &sheet(&[
                "roll = 30° # bank",
                "t = 0.25",
                "dt = 0.002",
                "k = -45°",
                "a = 0.5 rad",
            ]),
            &Live::default(),
        );
        let degrees = rows[0].slider.unwrap();
        assert_eq!((degrees.unit, degrees.value), (Unit::Degrees, 30.0));
        assert_eq!(rows[1].slider.unwrap().unit, Unit::Plain);
        assert_eq!(rows[2].slider, None);
        assert_eq!(rows[3].slider.unwrap().value, -45.0);
        assert_eq!(rows[4].slider.unwrap().unit, Unit::Radians);
        assert_eq!(
            set_literal("roll = 30° # bank", 45.5).as_deref(),
            Some("roll = 45.5° # bank")
        );
        assert_eq!(set_literal("t = 0.25", 0.3).as_deref(), Some("t = 0.3"));
        assert_eq!(
            set_literal("a = 0.5 rad", -1.0).as_deref(),
            Some("a = -1 rad")
        );
        assert_eq!(set_literal("q = euler(1, 2, 3)", 1.0), None);
    }

    #[test]
    fn check_names_the_convention_a_value_crossed() {
        let example = EXAMPLES.iter().find(|e| e.id == "check").unwrap();
        let rows = evaluate(&example.sheet(), &Live::default());
        let Some(Ok(Value::Verdict(v))) = rows[2].value else {
            panic!("{:?}", rows[2].value);
        };
        assert_eq!(v.relation, Relation::WLast);
    }

    /// Every example evaluates clean — the live one with a firmware and a
    /// plant to read — since an example that opens on a refusal teaches the
    /// wrong thing on the first click.
    #[test]
    fn every_example_evaluates_without_a_problem() {
        let mut live = Live::default();
        for name in ["roll", "pitch", "yaw", "gx", "gy", "gz"] {
            live.channels.insert(name.into(), 0.1);
        }
        live.truth = Some(Quat::from_euler(crate::spatial::Euler::new(0.1, 0.1, 0.1)));
        for example in EXAMPLES {
            for (i, row) in evaluate(&example.sheet(), &live).into_iter().enumerate() {
                assert!(
                    matches!(row.value, Some(Ok(_)) | None),
                    "{}: row {i} → {:?}",
                    example.id,
                    row.value
                );
            }
        }
        let gravity = EXAMPLES.iter().find(|e| e.id == "gravity").unwrap();
        let rows = evaluate(&gravity.sheet(), &live);
        let Some(Ok(Value::Verdict(v))) = rows[5].value else {
            panic!("{:?}", rows[5].value);
        };
        assert_eq!(v.relation, Relation::Same);
    }
}
