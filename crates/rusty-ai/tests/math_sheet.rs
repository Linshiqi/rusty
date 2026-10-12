//! `math_sheet`: the Math panel's evaluator, answering a model.
//!
//! A model relays what a tool hands it with total confidence, so these
//! check the payload a model would read — the frame an attitude was worked
//! out in, what it looks like, which convention a check named — and that
//! every refusal says what to change.

use std::{fs, path::Path};

use rusty_ai::{ToolContext, ToolRegistry};
use rusty_embed::spatial::sheet::Live;
use rusty_embed::spatial::{Quat, Vec3};
use serde_json::{Value, json};

fn call(args: Value, ctx: &ToolContext<'_>) -> Value {
    ToolRegistry::workbench()
        .call("math_sheet", &args, ctx)
        .unwrap_or_else(|e| panic!("math_sheet failed: {e}"))
}

fn refused(args: Value, ctx: &ToolContext<'_>) -> String {
    ToolRegistry::workbench()
        .call("math_sheet", &args, ctx)
        .map(|answer| panic!("expected a refusal, got {answer}"))
        .unwrap_err()
        .to_string()
}

fn at(root: &Path) -> ToolContext<'_> {
    ToolContext {
        root: Some(root),
        ..ToolContext::empty()
    }
}

fn close(value: &Value, want: f64) -> bool {
    value.as_f64().is_some_and(|got| (got - want).abs() < 1e-6)
}

fn sheet_in(dir: &Path, text: &str) {
    fs::create_dir_all(dir.join(".rusty")).unwrap();
    fs::write(dir.join(".rusty/math.toml"), text).unwrap();
}

/// The same three angles are two physical attitudes: a positive pitch is
/// the nose down with Z up and up with Z down. The instrument says which,
/// so a model describing the attitude has the words and not a sign to
/// guess from.
#[test]
fn an_attitude_comes_back_with_what_it_looks_like_in_its_frame() {
    let empty = ToolContext::empty();
    let up = call(
        json!({ "rows": ["q = euler(0°, 10°, 0°)"], "frame": "z-up" }),
        &empty,
    );
    let q = &up["rows"][0]["value"];
    assert_eq!(q["kind"], "quaternion", "{q}");
    assert!(close(&q["eulerDegrees"]["pitch"], 10.0), "{q}");
    assert!(close(&q["instrument"]["noseUpDegrees"], -10.0), "{q}");
    assert_eq!(up["frame"], "z-up");
    assert!(
        up["frameMeaning"]
            .as_str()
            .unwrap()
            .contains("puts the nose down")
    );

    let down = call(
        json!({ "rows": ["q = euler(0°, 10°, 0°)"], "frame": "z-down" }),
        &empty,
    );
    assert!(close(
        &down["rows"][0]["value"]["instrument"]["noseUpDegrees"],
        10.0
    ));

    // The working shows the order the three turns are made in, which is
    // the thing people get wrong.
    let steps: Vec<&str> = up["rows"][0]["working"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["step"].as_str().unwrap())
        .collect();
    assert_eq!(
        steps,
        [
            "1. Yaw about Z",
            "2. Pitch about the new Y",
            "3. Roll about the newest X"
        ]
    );
    assert!(up["conventions"].as_str().unwrap().contains("w first"));
}

/// "Differs by 108°" sends somebody through their code; the name of the
/// crossed convention is the fix.
#[test]
fn check_names_the_convention_a_quaternion_crossed() {
    let answer = call(
        json!({ "rows": [
            "reference = euler(20°, -15°, 60°)",
            "mine = quat(0.2134, -0.0252, 0.5078, 0.8342)",
            "check(mine, reference)",
        ] }),
        &ToolContext::empty(),
    );
    let verdict = &answer["rows"][2]["value"];
    assert_eq!(verdict["kind"], "check", "{verdict}");
    assert_eq!(verdict["relation"], "w-last");
    assert!(
        verdict["meaning"]
            .as_str()
            .unwrap()
            .contains("w written last")
    );
    assert!(verdict["errorDegrees"].as_f64().unwrap() > 1.0);
}

#[test]
fn a_refusal_is_named_and_the_rows_after_it_still_answer() {
    let answer = call(
        json!({ "rows": [
            "a = b",
            "b = 2",
            "c = (1, 2, 3) * (4, 5, 6)",
            "d = (1, 0, 0, 0)",
            "e = b * 3",
            "f = euler(30, 0, 0)",
            "# a remark and nothing else",
        ] }),
        &ToolContext::empty(),
    );
    let rows = answer["rows"].as_array().unwrap();
    let kind = |i: usize| rows[i]["problem"]["kind"].as_str().unwrap_or("");
    assert_eq!(kind(0), "later");
    assert!(
        rows[0]["problem"]["text"]
            .as_str()
            .unwrap()
            .contains("top to bottom")
    );
    assert_eq!(kind(2), "vector-product");
    assert!(
        rows[2]["problem"]["text"]
            .as_str()
            .unwrap()
            .contains("cross(a, b)")
    );
    assert_eq!(kind(3), "four-tuple");
    assert!(close(&rows[4]["value"]["number"], 6.0));
    // Thirty radians where an angle goes was probably thirty degrees.
    assert_eq!(rows[5]["notes"][0]["kind"], "bare-angle");
    assert!(rows[6].get("value").is_none() && rows[6].get("problem").is_none());
    assert_eq!(answer["total"], 7);
    assert_eq!(answer["truncated"], false);
}

/// Without rows it is the user's own sheet, in the user's own frame — and
/// rows a model writes are worked out in that frame too unless it names
/// another, so the answer and the user's panel draw the same aircraft.
#[test]
fn the_project_sheet_is_worked_out_in_its_own_frame() {
    let dir = tempfile::tempdir().unwrap();
    sheet_in(
        dir.path(),
        "frame = \"z-down\"\nrows = [\n  \"q = euler(0°, 10°, 0°)\",\n  \"n = norm(q)\",\n]\n",
    );
    let ctx = at(dir.path());

    let own = call(json!({}), &ctx);
    assert_eq!(own["source"], ".rusty/math.toml");
    assert_eq!(own["frame"], "z-down");
    assert_eq!(own["frameFrom"], "the project's sheet");
    assert_eq!(own["rows"][0]["text"], "q = euler(0°, 10°, 0°)");
    assert!(close(
        &own["rows"][0]["value"]["instrument"]["noseUpDegrees"],
        10.0
    ));
    assert!(close(&own["rows"][1]["value"]["number"], 1.0));

    let written = call(json!({ "rows": ["p = euler(0°, 5°, 0°)"] }), &ctx);
    assert_eq!(written["source"], "rows");
    assert_eq!(written["frame"], "z-down");
    assert_eq!(written["frameFrom"], "the project's sheet");

    let named = call(
        json!({ "rows": ["p = euler(0°, 5°, 0°)"], "frame": "z-up" }),
        &ctx,
    );
    assert_eq!(named["frame"], "z-up");
    assert_eq!(named["frameFrom"], "the call");
}

#[test]
fn a_project_with_no_sheet_says_so_rather_than_inventing_one() {
    let dir = tempfile::tempdir().unwrap();
    let answer = call(json!({}), &at(dir.path()));
    assert_eq!(answer["exists"], false);
    assert!(answer["note"].as_str().unwrap().contains("`rows`"));
    assert!(answer.get("rows").is_none());

    // Rows are worked out in the Math panel's default frame, and the answer
    // says it is the default.
    let rows = call(json!({ "rows": ["a = 1"] }), &at(dir.path()));
    assert_eq!(rows["frame"], "z-up");
    assert!(
        rows["frameFrom"]
            .as_str()
            .unwrap()
            .starts_with("the default")
    );
}

#[test]
fn with_nothing_open_and_no_rows_it_says_what_to_pass() {
    let reason = refused(json!({}), &ToolContext::empty());
    assert!(reason.contains("`rows`"), "{reason}");
}

/// A sheet that does not read is named, never read as an empty one — and a
/// frame it would have supplied is asked for rather than assumed.
#[test]
fn an_unreadable_sheet_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    sheet_in(dir.path(), "rows = [\n");
    let ctx = at(dir.path());

    let reason = refused(json!({}), &ctx);
    assert!(reason.contains(".rusty/math.toml"), "{reason}");

    let reason = refused(json!({ "rows": ["a = 1"] }), &ctx);
    assert!(reason.contains("pass `frame`"), "{reason}");

    // Given the frame, the file is not needed.
    let answer = call(json!({ "rows": ["a = 1"], "frame": "z-up" }), &ctx);
    assert!(close(&answer["rows"][0]["value"]["number"], 1.0));
}

/// The window sends what a running simulation says with the question, so a
/// sheet the Math panel shows live is worked out with the same numbers;
/// values a caller passes win over it, and a channel nobody has is said.
#[test]
fn live_rows_read_the_window_and_telemetry_wins() {
    let mut live = Live::default();
    live.channels.insert("roll".into(), 0.5);
    live.truth = Some(Quat::IDENTITY);
    let ctx = ToolContext {
        live: Some(&live),
        ..ToolContext::empty()
    };
    let rows = json!(["r = tel(\"roll\")", "p = tel(\"pitch\")", "t = truth()"]);

    let answer = call(json!({ "rows": rows }), &ctx);
    assert!(close(&answer["rows"][0]["value"]["number"], 0.5));
    assert_eq!(answer["rows"][0]["live"], true);
    assert_eq!(answer["rows"][1]["problem"]["kind"], "no-channel");
    assert_eq!(answer["rows"][2]["value"]["kind"], "quaternion");
    assert!(answer["live"].as_str().unwrap().contains("Math panel"));

    let given = call(
        json!({ "rows": rows, "telemetry": { "roll": 0.25, "pitch": -0.1 } }),
        &ctx,
    );
    assert!(close(&given["rows"][0]["value"]["number"], 0.25));
    assert!(close(&given["rows"][1]["value"]["number"], -0.1));

    // Outside the app nothing runs, and the plant is said to be absent.
    let bare = call(json!({ "rows": rows }), &ToolContext::empty());
    assert_eq!(bare["rows"][2]["problem"]["kind"], "no-plant");
    assert!(bare["live"].as_str().unwrap().starts_with("none"));
}

/// The window's snapshot crosses to `ai_ask` as JSON — Tauri writes the
/// channel map as an object, and each attitude's parts by name — and has
/// to arrive as the values the panel read.
#[test]
fn the_window_snapshot_survives_the_wire() {
    let mut live = Live::default();
    live.channels.insert("roll".into(), 0.25);
    live.truth = Some(Quat::new(0.9, 0.1, 0.2, 0.3));
    live.truth_rate = Some(Vec3::new(0.1, -0.2, 0.3));
    let wire = serde_json::to_value(&live).unwrap();
    assert_eq!(wire["channels"]["roll"], 0.25);
    assert_eq!(wire["truth"]["w"], 0.9);
    assert_eq!(wire["truthRate"]["y"], -0.2);
    assert_eq!(serde_json::from_value::<Live>(wire).unwrap(), live);

    // A window with no plant running sends the channels alone.
    let bare: Live = serde_json::from_value(json!({ "channels": { "gx": 1.5 } })).unwrap();
    assert_eq!(bare.channels["gx"], 1.5);
    assert!(bare.truth.is_none() && bare.truth_rate.is_none());
}

#[test]
fn arguments_that_cannot_be_read_are_refused_by_name() {
    let empty = ToolContext::empty();
    assert!(refused(json!({ "rows": [] }), &empty).contains("`rows` is empty"));
    assert!(refused(json!({ "rows": [1, 2] }), &empty).contains("string"));
    assert!(refused(json!({ "rows": ["a = 1"], "frame": "up" }), &empty).contains("`frame`"));
    assert!(
        refused(
            json!({ "rows": ["a = 1"], "telemetry": { "roll": "fast" } }),
            &empty
        )
        .contains("telemetry.roll")
    );
    // One string is a row a line.
    let lines = call(json!({ "rows": "a = 1\nb = a + 1" }), &empty);
    assert!(close(&lines["rows"][1]["value"]["number"], 2.0));
}

/// `f64` leaves rounding noise where a zero belongs; read out, it is a
/// number a model would repeat.
#[test]
fn noise_where_a_zero_belongs_is_not_read_out() {
    let answer = call(
        json!({ "rows": ["v = rotate(axis_angle(Z, 90°), X)", "R = dcm(identity)"] }),
        &ToolContext::empty(),
    );
    let v = &answer["rows"][0]["value"];
    assert_eq!(v["x"], json!(0.0), "{v}");
    assert_eq!(v["y"], json!(1.0), "{v}");
    let r = &answer["rows"][1]["value"];
    assert_eq!(r["kind"], "matrix");
    assert_eq!(
        r["rows"],
        json!([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
    );
    assert!(close(&r["det"], 1.0));
}

#[test]
fn the_working_can_be_left_out() {
    let answer = call(
        json!({ "rows": ["q = euler(10°, 20°, 30°)"], "working": false }),
        &ToolContext::empty(),
    );
    assert!(answer["rows"][0].get("working").is_none());
    assert_eq!(answer["rows"][0]["value"]["kind"], "quaternion");
}
