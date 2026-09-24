//! The math toolbox's sheet, worked out for a model.
//!
//! Rotation arithmetic is where a model is most fluently wrong. Attitude
//! code is seldom nearly right — it is right with one convention crossed:
//! `w` first or last, body-to-world or its inverse, Z-Y-X or X-Y-Z, Z up or
//! Z down, degrees for radians — and a quaternion multiplied out from memory
//! crosses one without a word, then reads as a perfectly plausible number.
//! So the model does not multiply; it writes the rows the user would write in
//! the Math panel and gets back what the panel shows: every value, the
//! working with the numbers in, and every refusal by name.
//!
//! The same evaluator as the panel's (`rusty_embed::spatial::sheet`), so the
//! assistant and the page cannot disagree about a number. What the panel
//! says in the reader's language this says in English, for the model — the
//! arithmetic names what happened, and both word it.

use std::collections::HashMap;

use serde_json::{Map, Value as Json, json};

use rusty_embed::spatial::check::Relation;
use rusty_embed::spatial::instrument;
use rusty_embed::spatial::sheet::steps::{deg, euler, mat3, num, quat, vec2, vec3};
use rusty_embed::spatial::sheet::{
    Kind, Live, MathSheet, Note, Problem, Remark, Row, Step, Value, What, evaluate,
};
use rusty_embed::spatial::{Euler, Frame, Quat, sheet_file};

use super::{Tool, ToolContext, bool_arg, read_only};
use crate::{
    error::{Error, Result},
    model::ToolDef,
};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(Sheet)]
}

const NAME: &str = "math_sheet";

/// Rows an answer carries at most. A sheet somebody types is a screenful;
/// past this the rows are still worked out, so the ones returned see every
/// row above them, and the answer says it stopped.
const MAX_ROWS: usize = 200;

/// The conventions every number in an answer is in, sent with the numbers
/// so a model relaying them has them to hand.
const CONVENTIONS: &str = "Quaternions are Hamilton's, written w first, and turn the \
    body's axes onto the world's: rotate(q, v) = q ⊗ v ⊗ q* takes a body vector into \
    the world. a * b turns by b first, then by a. Euler angles are Z-Y-X intrinsic: yaw \
    about Z, then pitch about the new Y, then roll about the newest X. Angles are shown \
    in degrees and carried in radians. `instrument` is what an attitude indicator \
    shows — the nose's elevation above the horizon, the bank (right wing down is \
    positive) and the heading clockwise from world X seen from above — read off where \
    the body's axes point, so it means the same in both frames, which the Euler \
    angles' signs do not.";

struct Sheet;

impl Tool for Sheet {
    fn def(&self) -> ToolDef {
        read_only(
            NAME,
            "Work out rows of rusty's math sheet — the attitude arithmetic a flight \
             controller is made of — exactly, with the working shown: quaternions, Euler \
             angles, rotation matrices, vectors, gravity in the body, gyro integration, \
             and `check`, which names the convention a wrong attitude crossed. Without \
             `rows` it works out the open project's own sheet (.rusty/math.toml, what \
             the user sees in the Math panel), with the values a running simulation \
             gives it where the app has them. \
             \
             Call this instead of doing rotation arithmetic in your head, and before \
             saying what an attitude looks like. Attitude code is seldom nearly right: \
             it is right with one convention crossed — w first or last, body-to-world \
             or its inverse, Z-Y-X or X-Y-Z, Z up or Z down, degrees for radians — and \
             arithmetic from memory crosses one silently. Each row comes back with its \
             value (an attitude also as Euler angles, axis and angle, and `instrument`: \
             nose up, bank and heading, which mean the same in both frames while the \
             angles' signs do not), the working of each operation with the numbers in, \
             and whatever was refused, by name. \
             \
             The language: one `name = expression` a row, read top to bottom, each \
             seeing the rows above; `#` starts a remark. Angles take a unit — `30°`, \
             `30 deg`, `0.5 rad` — and a plain number is radians. `(x, y, z)` is a \
             vector and `(x, y)` a plane vector; four numbers in brackets are refused \
             as ambiguous, write quat(w, x, y, z). Constants: X Y Z identity pi. \
             Build: euler(roll, pitch, yaw), quat(w, x, y, z), axis_angle(axis, angle), \
             from_rotvec(v), from_to(a, b), from_dcm(R). Read: to_euler(q), dcm(q), \
             axis(q), angle(q), to_rotvec(q), norm, normalize, and fields q.w q.x q.y \
             q.z, v.x v.y v.z, e.roll e.pitch e.yaw. Turn: to_world(q, v) = rotate(q, v) \
             = q * v (body to world), to_body(q, v) (world to body), a * b, conj(q), \
             inv(q), delta(a, b) = a* ⊗ b, angle(a, b), slerp(a, b, t), integrate(q, w, \
             dt) (a body rate in rad/s, exactly) and integrate_linear(q, w, dt) (first \
             order and not normalised, as most firmware does it). Gravity: \
             accel_at_rest(q) (what an accelerometer reads at rest, in g, body axes), \
             gravity_body(q), tilt(acc) (roll and pitch back from a reading). Vectors: \
             dot, cross, project(a, b), rotate(angle, v) in the plane, mat(r0, r1, r2), \
             transpose, det, R * v. Numbers: deg(x), rad(x), sin cos tan asin acos atan \
             atan2 sqrt abs. check(mine, reference) takes two quaternions, Euler angles, \
             vectors, angles or numbers. tel(\"name\") is a telemetry channel's newest \
             value — give values in `telemetry`; truth() and truth_rate() are the \
             simulator's plant, there only while the app's Flight tab runs one. \
             \
             For example: [\"q = euler(30°, 10°, 45°)\", \"acc = accel_at_rest(q)\", \
             \"back = tilt(acc)\", \"check(quat(0.9, 0.2, 0.1, 0.3), q)\"].",
            json!({
                "type": "object",
                "properties": {
                    "rows": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Rows to work out, top to bottom, one `name = expression` each. Leave out to work out the open project's own sheet."
                    },
                    "frame": {
                        "type": "string",
                        "enum": ["z-up", "z-down"],
                        "description": "Which way the world's Z points: z-up (the body forward-left-up: Crazyflie, ROS) or z-down (north-east-down, the body forward-right-down: PX4, ArduPilot). Default: the frame of the project's sheet, else z-up."
                    },
                    "telemetry": {
                        "type": "object",
                        "description": "Numbers for tel(\"name\") by channel name, e.g. {\"roll\": 0.12, \"pitch\": -0.03} — the values of a [rusty:tel] line, say. They win over the running simulation's."
                    },
                    "working": {
                        "type": "boolean",
                        "description": "Include each operation's working (default true). Leave it on to explain a result; turn it off for a long sheet when only the values matter."
                    }
                },
                "required": []
            }),
        )
    }

    fn call(&self, args: &Json, ctx: &ToolContext<'_>) -> Result<Json> {
        let rows = rows_arg(args)?;
        let asked_frame = frame_arg(args)?;
        let telemetry = telemetry_arg(args)?;
        let working = bool_arg(args, "working", true);

        // The project's sheet: what is worked out when no rows are given,
        // and whose frame rows are worked out in when the call names none —
        // the frame the user's own panel draws in.
        let wanted_file = rows.is_none() || asked_frame.is_none();
        let file = match (ctx.root, wanted_file) {
            (Some(root), true) => sheet_file::load(root).map_err(|error| {
                Error::Refused(format!(
                    "{error}{}",
                    if rows.is_some() {
                        " — so the frame the user works in is not known: pass `frame`"
                    } else {
                        ""
                    }
                ))
            })?,
            _ => None,
        };

        let (source, sheet_rows, file_frame) = match rows {
            Some(rows) => ("rows", rows, file.as_ref().map(|s| s.frame)),
            None => {
                if ctx.root.is_none() {
                    return Err(Error::MissingContext {
                        needed: "an open project, or `rows` to work out".into(),
                        hint: "Pass the rows to evaluate; with no project open there is \
                               no sheet of the user's to read."
                            .into(),
                    });
                }
                let Some(sheet) = file else {
                    return Ok(json!({
                        "source": sheet_file::PATH,
                        "exists": false,
                        "note": "The project has no math sheet yet — nothing has been \
                                 written in its Math panel. Pass `rows` to work something out.",
                    }));
                };
                (sheet_file::PATH, sheet.rows, Some(sheet.frame))
            }
        };

        let (frame, frame_from) = match (asked_frame, file_frame) {
            (Some(frame), _) => (frame, "the call"),
            (None, Some(frame)) => (frame, "the project's sheet"),
            (None, None) => (Frame::ZUp, "the default: the project has no sheet"),
        };

        let snapshot = ctx.live.filter(|live| !is_empty(live));
        let mut live = snapshot.cloned().unwrap_or_default();
        let given = !telemetry.is_empty();
        live.channels.extend(telemetry);
        let live_from = match (snapshot.is_some(), given) {
            (true, true) => {
                "the simulation's values as the window held them when the question was \
                 asked — what the Math panel's live rows show — with `telemetry` over them"
            }
            (true, false) => {
                "the simulation's values as the window held them when the question was \
                 asked — what the Math panel's live rows show"
            }
            (false, true) => "`telemetry`",
            (false, false) => {
                "none: no simulation's values reached this call, and no `telemetry` was given"
            }
        };

        let sheet = MathSheet {
            frame,
            rows: sheet_rows,
        };
        let worked = evaluate(&sheet, &live);
        let total = worked.len();
        let out: Vec<Json> = worked
            .iter()
            .zip(&sheet.rows)
            .take(MAX_ROWS)
            .enumerate()
            .map(|(i, (row, text))| row_json(i, text, row, frame, working))
            .collect();

        Ok(json!({
            "source": source,
            "frame": frame_name(frame),
            "frameMeaning": frame_meaning(frame),
            "frameFrom": frame_from,
            "live": live_from,
            "conventions": CONVENTIONS,
            "rows": out,
            "total": total,
            "truncated": total > MAX_ROWS,
        }))
    }
}

// ─── arguments ───────────────────────────────────────────────────────────────

/// The rows given, or `None` for the project's sheet. One string is taken
/// a row a line, which is what it can only mean.
fn rows_arg(args: &Json) -> Result<Option<Vec<String>>> {
    match args.get("rows") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(text)) => Ok(Some(text.lines().map(str::to_string).collect())),
        Some(Json::Array(items)) if items.is_empty() => Err(Error::bad_args(
            NAME,
            "`rows` is empty: pass the rows to work out, or leave it out for the \
             project's own sheet",
        )),
        Some(Json::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_string).ok_or_else(|| {
                    Error::bad_args(NAME, "every item of `rows` must be a string, one row each")
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => Err(Error::bad_args(
            NAME,
            "`rows` must be an array of strings, one row each",
        )),
    }
}

fn frame_arg(args: &Json) -> Result<Option<Frame>> {
    match args.get("frame") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(name)) if name == "z-up" => Ok(Some(Frame::ZUp)),
        Some(Json::String(name)) if name == "z-down" => Ok(Some(Frame::ZDown)),
        Some(other) => Err(Error::bad_args(
            NAME,
            format!("`frame` is \"z-up\" or \"z-down\", not {other}"),
        )),
    }
}

fn telemetry_arg(args: &Json) -> Result<HashMap<String, f64>> {
    match args.get("telemetry") {
        None | Some(Json::Null) => Ok(HashMap::new()),
        Some(Json::Object(channels)) => channels
            .iter()
            .map(|(name, value)| {
                value
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .map(|v| (name.clone(), v))
                    .ok_or_else(|| {
                        Error::bad_args(NAME, format!("`telemetry.{name}` must be a number"))
                    })
            })
            .collect(),
        Some(_) => Err(Error::bad_args(
            NAME,
            "`telemetry` must be an object of channel names to numbers, such as \
             {\"roll\": 0.12}",
        )),
    }
}

fn is_empty(live: &Live) -> bool {
    live.channels.is_empty() && live.truth.is_none() && live.truth_rate.is_none()
}

// ─── the answer ──────────────────────────────────────────────────────────────

fn frame_name(frame: Frame) -> &'static str {
    match frame {
        Frame::ZUp => "z-up",
        Frame::ZDown => "z-down",
    }
}

fn frame_meaning(frame: Frame) -> &'static str {
    match frame {
        Frame::ZUp => {
            "World Z up; the body X forward, Y left, Z up (FLU: Crazyflie, ROS, \
             cf-drone-rs). A positive pitch puts the nose down and a positive yaw turns \
             it left; a level accelerometer reads (0, 0, 1)."
        }
        Frame::ZDown => {
            "World Z down (north-east-down); the body X forward, Y right, Z down (FRD: \
             PX4, ArduPilot). A positive pitch puts the nose up and a positive yaw turns \
             it right; a level accelerometer reads (0, 0, -1)."
        }
    }
}

/// A number as an answer carries it: to nine decimals, past anything a
/// firmware's `f32` can say and short of the rounding noise `f64` leaves
/// where there should be a zero — which a model would otherwise read out.
fn figure(v: f64) -> Json {
    if !v.is_finite() {
        return Json::Null;
    }
    if v.abs() >= 1e6 {
        return json!(v);
    }
    let r = (v * 1e9).round() / 1e9;
    json!(if r == 0.0 { 0.0 } else { r })
}

fn degrees(radians: f64) -> Json {
    figure(radians.to_degrees())
}

fn row_json(index: usize, text: &str, row: &Row, frame: Frame, working: bool) -> Json {
    let mut out = Map::new();
    out.insert("row".into(), json!(index + 1));
    out.insert("text".into(), json!(text));
    if let Some(name) = &row.name {
        out.insert("name".into(), json!(name));
    }
    match &row.value {
        Some(Ok(value)) => {
            out.insert("value".into(), value_json(value, frame));
        }
        Some(Err(refused)) => {
            let (kind, text) = problem(refused);
            out.insert("problem".into(), json!({ "kind": kind, "text": text }));
        }
        // Nothing on it but a remark.
        None => {}
    }
    if !row.notes.is_empty() {
        let notes: Vec<Json> = row
            .notes
            .iter()
            .map(|n| {
                let (kind, text) = note(n);
                json!({ "kind": kind, "text": text })
            })
            .collect();
        out.insert("notes".into(), json!(notes));
    }
    if row.live {
        out.insert("live".into(), json!(true));
    }
    if working && !row.steps.is_empty() {
        let steps: Vec<Json> = row.steps.iter().map(step_json).collect();
        out.insert("working".into(), json!(steps));
    }
    Json::Object(out)
}

fn step_json(step: &Step) -> Json {
    let mut out = Map::new();
    out.insert("step".into(), json!(what(step.what)));
    out.insert("lines".into(), json!(step.lines));
    if !step.remarks.is_empty() {
        let remarks: Vec<&str> = step.remarks.iter().map(|r| remark(*r)).collect();
        out.insert("remarks".into(), json!(remarks));
    }
    Json::Object(out)
}

fn value_json(value: &Value, frame: Frame) -> Json {
    match value {
        Value::Number(v) => json!({ "kind": "number", "number": figure(*v), "text": num(*v) }),
        Value::Angle(a) => json!({
            "kind": "angle",
            "degrees": degrees(*a),
            "radians": figure(*a),
            "text": format!("{} ({} rad)", deg(*a), num(*a)),
        }),
        Value::Vec2(v) => json!({
            "kind": "vec2",
            "x": figure(v.x),
            "y": figure(v.y),
            "length": figure(v.norm()),
            "text": vec2(*v),
        }),
        Value::Vec3(v) => json!({
            "kind": "vec3",
            "x": figure(v.x),
            "y": figure(v.y),
            "z": figure(v.z),
            "length": figure(v.norm()),
            "text": vec3(*v),
        }),
        Value::Quat(q) => attitude(*q, None, frame),
        Value::Euler(e) => attitude(Quat::from_euler(*e), Some(*e), frame),
        Value::Mat3(m) => json!({
            "kind": "matrix",
            "rows": m.m.iter().map(|r| r.iter().map(|v| figure(*v)).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "det": figure(m.det()),
            "orthonormalError": figure(m.orthonormal_error()),
            "text": mat3(*m),
        }),
        Value::Verdict(v) => {
            let (kind, meaning) = relation(v.relation);
            let mut out = json!({
                "kind": "check",
                "relation": kind,
                "meaning": meaning,
            });
            if v.angle {
                out["errorDegrees"] = degrees(v.error);
            } else {
                out["error"] = figure(v.error);
            }
            out
        }
    }
}

/// An attitude as the Math panel's readout gives it: its quaternion, its
/// Euler angles, its axis and angle, and what the instrument would show.
/// A quaternion off the unit sphere is read as the attitude its direction
/// is, and says its length; one of no length is no attitude.
fn attitude(q: Quat, given: Option<Euler>, frame: Frame) -> Json {
    let mut out = Map::new();
    let is_euler = given.is_some();
    out.insert(
        "kind".into(),
        json!(if is_euler { "euler" } else { "quaternion" }),
    );
    let text = match given {
        Some(e) => euler(e),
        None => quat(q),
    };
    out.insert("text".into(), json!(text));
    let parts = json!({ "w": figure(q.w), "x": figure(q.x), "y": figure(q.y), "z": figure(q.z) });
    if is_euler {
        out.insert("quaternion".into(), parts);
    } else {
        for key in ["w", "x", "y", "z"] {
            out.insert(key.into(), parts[key].clone());
        }
        out.insert("norm".into(), figure(q.norm()));
    }
    let Some(unit) = q.normalized() else {
        return Json::Object(out);
    };
    let e = given.unwrap_or_else(|| unit.to_euler());
    out.insert(
        "eulerDegrees".into(),
        json!({ "roll": degrees(e.roll), "pitch": degrees(e.pitch), "yaw": degrees(e.yaw) }),
    );
    if is_euler {
        out.insert(
            "eulerRadians".into(),
            json!({ "roll": figure(e.roll), "pitch": figure(e.pitch), "yaw": figure(e.yaw) }),
        );
    }
    match unit.axis_angle() {
        Some((axis, angle)) => {
            out.insert(
                "axis".into(),
                json!({ "x": figure(axis.x), "y": figure(axis.y), "z": figure(axis.z) }),
            );
            out.insert("angleDegrees".into(), degrees(angle));
        }
        None => {
            out.insert("angleDegrees".into(), json!(0.0));
        }
    }
    let reading = instrument::read(unit, frame);
    out.insert(
        "instrument".into(),
        json!({
            "noseUpDegrees": degrees(reading.nose_up),
            "bankRightDegrees": degrees(reading.bank_right),
            "headingDegrees": degrees(reading.heading),
        }),
    );
    Json::Object(out)
}

// ─── the words ───────────────────────────────────────────────────────────────
//
// The panel's catalogue says these in the reader's language
// (`[math.*]` in rusty-i18n); these say them to a model. Where the panel
// points at a button, these point at an argument.

fn what(w: What) -> &'static str {
    match w {
        What::Add => "Adding vectors",
        What::Sub => "Subtracting vectors",
        What::Scale => "Scaling a vector",
        What::Negate => "Negating",
        What::Dot => "Dot product",
        What::Cross => "Cross product",
        What::Norm => "Length",
        What::Normalize => "Normalising",
        What::Project => "Projecting",
        What::AngleBetween => "The angle between two vectors",
        What::Turn2 => "Turning in the plane",
        What::Quaternion => "The quaternion's axis and angle",
        What::AxisAngle => "A turn about an axis",
        What::EulerYaw => "1. Yaw about Z",
        What::EulerPitch => "2. Pitch about the new Y",
        What::EulerRoll => "3. Roll about the newest X",
        What::ToEuler => "Reading Euler angles (Z-Y-X)",
        What::ToEulerPole => "Reading Euler angles at a pole",
        What::Matrix => "Rotation matrix",
        What::FromMatrix => "Quaternion from a matrix",
        What::Compose => "Composing two rotations",
        What::Conjugate => "Conjugate",
        What::Inverse => "Inverse",
        What::ToWorld => "Body to world: q ⊗ v ⊗ q*",
        What::ToBody => "World to body: q* ⊗ v ⊗ q",
        What::Rodrigues => "The same turn as Rodrigues' three vectors",
        What::Slerp => "Slerp",
        What::FromTo => "The shortest turn between two directions",
        What::Integrate => "One gyro step, exactly",
        What::IntegrateLinear => "One gyro step, to first order",
        What::Delta => "The turn between two attitudes",
        What::AttitudeError => "Attitude error",
        What::ToRotvec => "Rotation vector",
        What::FromRotvec => "From a rotation vector",
        What::AtRest => "An accelerometer at rest",
        What::Gravity => "Gravity in the body",
        What::Tilt => "Tilt from an accelerometer",
        What::MatVec => "Matrix times vector",
        What::MatMat => "Matrix times matrix",
        What::Transpose => "Transpose",
        What::Det => "Determinant",
        What::Check => "Check",
    }
}

fn remark(r: Remark) -> &'static str {
    match r {
        Remark::Area => {
            "|a × b| is the area of the parallelogram a and b span, and a × b stands at \
             right angles to both."
        }
        Remark::ShortWay => "The angle is the shorter way round; −q names the same turn.",
        Remark::NoTurn => "No turn, so no axis.",
        Remark::Flipped => {
            "The dot product was negative, so b was negated: the same attitude, reached \
             the short way."
        }
        Remark::Columns => "The columns are the body's X, Y and Z seen from the world.",
        Remark::BodyRate => "The rate is in the body's axes, so the step multiplies on the right.",
        Remark::OffSphere => {
            "To first order the quaternion leaves the unit sphere — which is why firmware \
             normalises after every step."
        }
        Remark::OwnAxes => {
            "a* ⊗ b is the turn from a to b in a's own axes: what a controller closes."
        }
        Remark::NoHeading => {
            "Gravity says nothing about heading: yaw cannot be read off an accelerometer."
        }
        Remark::TransposeInverse => "A rotation's transpose is its inverse.",
        Remark::DetSign => "The determinant is 1 for a rotation and −1 for a mirror.",
        Remark::Pole => {
            "At a pole roll and yaw turn about one axis: roll is set to zero and the rest \
             given to yaw."
        }
        Remark::Antiparallel => {
            "Opposite directions: any perpendicular axis turns one onto the other."
        }
        Remark::BodyAxes => "The reading is in the body's axes, in g.",
        Remark::SameAttitude => "The two share an attitude and differ only in sign.",
    }
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Number => "a number",
        Kind::Angle => "an angle",
        Kind::Vec2 => "a plane vector",
        Kind::Vec3 => "a vector",
        Kind::Quat => "a quaternion",
        Kind::Euler => "Euler angles",
        Kind::Mat3 => "a matrix",
        Kind::Verdict => "a check",
        Kind::Text => "a quoted name",
    }
}

fn kinds(list: &[Kind]) -> String {
    if list.is_empty() {
        return "nothing".into();
    }
    list.iter().map(|k| kind(*k)).collect::<Vec<_>>().join(", ")
}

/// A refusal's stable name — the panel's catalogue key — and its sentence.
fn problem(p: &Problem) -> (&'static str, String) {
    match p {
        Problem::Unexpected { found, at } => (
            "unexpected",
            format!("“{found}” has no place here (character {}).", at + 1),
        ),
        Problem::Unclosed { at } => (
            "unclosed",
            format!(
                "The bracket or quote at character {} is never closed.",
                at + 1
            ),
        ),
        Problem::Incomplete => ("incomplete", "The row ends where more was needed.".into()),
        Problem::BadNumber { text } => ("bad-number", format!("“{text}” is not a number.")),
        Problem::Unknown { name } => (
            "unknown",
            format!("“{name}” is not a row above, a constant or a function."),
        ),
        Problem::Later { name } => (
            "later",
            format!("“{name}” is defined below this row; the sheet is read top to bottom."),
        ),
        Problem::Twice { name } => (
            "twice",
            format!("“{name}” is already the name of a row above."),
        ),
        Problem::Reserved { name } => (
            "reserved",
            format!("“{name}” is a function; give the row another name."),
        ),
        Problem::Broken { name } => ("broken", format!("The row “{name}” above has no value.")),
        Problem::NotCallable { name } => (
            "not-callable",
            format!(
                "“{name}” cannot be used that way: a function needs brackets, and a value \
                 cannot be called."
            ),
        ),
        // A vector times a vector is the one people reach for most, and
        // what they meant is one of two functions.
        Problem::Operands {
            op: '*',
            left: Kind::Vec2 | Kind::Vec3,
            right: Kind::Vec2 | Kind::Vec3,
        } => (
            "vector-product",
            "A vector times a vector: write dot(a, b) or cross(a, b).".into(),
        ),
        Problem::Operands { op, left, right } => {
            let op = if *op == 'r' {
                "rad".to_string()
            } else {
                op.to_string()
            };
            (
                "operands",
                format!(
                    "{op} has no meaning between {} and {}.",
                    kind(*left),
                    kind(*right)
                ),
            )
        }
        Problem::Arguments { function, got } => (
            "arguments",
            format!("{function} does not take {}.", kinds(got)),
        ),
        Problem::NoField { kind: k, field } => {
            ("no-field", format!("{} has no .{field}.", kind(*k)))
        }
        Problem::FourTuple => (
            "four-tuple",
            "Four numbers could be (w, x, y, z) or (x, y, z, w): write quat(w, x, y, z).".into(),
        ),
        Problem::Tuple { len } => (
            "tuple",
            format!(
                "Brackets hold two or three numbers, a vector; these hold {len} things, or \
                 something that is not a number."
            ),
        ),
        Problem::NoDirection => (
            "no-direction",
            "That needs a direction, and something of no length has none.".into(),
        ),
        Problem::DivideByZero => ("divide-by-zero", "Division by zero.".into()),
        Problem::NotFinite => ("not-finite", "The result is not a number.".into()),
        Problem::NoChannel { name } => (
            "no-channel",
            format!(
                "No value for the telemetry channel “{name}”: nothing running has printed it, \
                 and `telemetry` does not give it."
            ),
        ),
        Problem::NoPlant => (
            "no-plant",
            "truth() and truth_rate() read the simulator's plant, which runs only while the \
             app's Flight tab closes the loop — none did as this was asked. Write the \
             attitude as quat(w, x, y, z) instead."
                .into(),
        ),
        Problem::StrayText => (
            "stray-text",
            "A quoted name only means something inside tel(\"…\").".into(),
        ),
    }
}

fn note(n: &Note) -> (&'static str, String) {
    match n {
        Note::NotUnit { norm } => (
            "not-unit",
            format!("|q| = {norm:.6}: not a unit quaternion, so it scales what it turns by |q|²."),
        ),
        Note::BareAngle { value } => (
            "bare-angle",
            format!(
                "{value} is taken as radians ({:.1}°); write {value}° for degrees.",
                value.to_degrees()
            ),
        ),
        Note::Pole => (
            "pole",
            "At a pole: pitch is ±90°, and roll and yaw turn about one axis.".into(),
        ),
    }
}

/// What `check` found — its stable name and what it means.
fn relation(r: Relation) -> (&'static str, &'static str) {
    match r {
        Relation::Same => ("same", "The same."),
        Relation::OtherSign => (
            "other-sign",
            "The same attitude with the other sign: q and −q are one rotation.",
        ),
        Relation::Inverse => (
            "inverse",
            "The inverse: world-to-body where body-to-world was meant, or a JPL quaternion \
             read as Hamilton's.",
        ),
        Relation::WLast => (
            "w-last",
            "The reference with w written last: (x, y, z, w) read as (w, x, y, z).",
        ),
        Relation::OrderXyz => (
            "order-xyz",
            "The same angles applied roll first (X-Y-Z) instead of yaw first (Z-Y-X).",
        ),
        Relation::OtherFrame => (
            "other-frame",
            "The same attitude in the other frame: pitch and yaw (or Y and Z) with the other \
             sign.",
        ),
        Relation::Opposite => ("opposite", "Pointing the opposite way."),
        Relation::Degrees => (
            "degrees",
            "Degrees where radians were meant: 57.3 times too big.",
        ),
        Relation::Radians => (
            "radians",
            "Radians where degrees were meant: 57.3 times too small.",
        ),
        Relation::Unexplained => (
            "unexplained",
            "Different, and not by any of the usual mistakes.",
        ),
    }
}
