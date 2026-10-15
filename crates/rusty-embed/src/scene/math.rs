//! The math toolbox's view, composed: the turns a row plays, which
//! attitude the aircraft is drawn at, which step's working is drawn, and
//! every item of space or of the plane with how strongly to draw it. Pure,
//! so the tests say which picture of a sheet the window draws.

use super::{Ink, Item, model, sketch::PALETTE};
use crate::spatial::sheet::{Row, Shape, Step, Value};
use crate::spatial::{Frame, Quat, Vec3};

/// The turns a row's steps make, in order: what playing it plays.
pub fn turns(row: &Row) -> Vec<(usize, (Quat, Quat))> {
    row.steps
        .iter()
        .enumerate()
        .filter_map(|(i, step)| step.turn.map(|turn| (i, turn)))
        .collect()
}

/// Where the turns have got to at `progress` (0 to 1 over all of them), and
/// which step that is.
pub fn attitude_at(turns: &[(usize, (Quat, Quat))], progress: f64) -> Option<(usize, Quat)> {
    if turns.is_empty() {
        return None;
    }
    let n = turns.len() as f64;
    let p = progress.clamp(0.0, 1.0) * n;
    let k = (p.floor() as usize).min(turns.len() - 1);
    let (step, (from, to)) = turns[k];
    Some((step, from.slerp(to, p - k as f64)))
}

/// The step whose working is drawn: the one being read, else the one the
/// turns are playing, else the last that has anything to draw.
pub fn active_step(row: &Row, chosen: Option<usize>, playing_at: Option<usize>) -> Option<usize> {
    chosen
        .filter(|i| *i < row.steps.len())
        .or(playing_at)
        .or_else(|| row.steps.iter().rposition(|s: &Step| !s.shapes.is_empty()))
}

/// The attitude the aircraft is drawn at: the selected row's when it has
/// one, else the last shown row that is one.
pub fn attitude_row(rows: &[Row], hidden: &[bool], selected: Option<usize>) -> Option<usize> {
    let is_attitude =
        |row: &Row| matches!(row.value, Some(Ok(Value::Quat(_)) | Ok(Value::Euler(_))));
    if let Some(i) = selected
        && rows.get(i).is_some_and(is_attitude)
    {
        return Some(i);
    }
    rows.iter()
        .enumerate()
        .rev()
        .find(|(i, row)| !hidden.get(*i).copied().unwrap_or(false) && is_attitude(row))
        .map(|(i, _)| i)
}

fn attitude_of(value: &Value) -> Option<Quat> {
    match value {
        Value::Quat(q) => q.normalized(),
        Value::Euler(e) => Some(Quat::from_euler(*e)),
        _ => None,
    }
}

/// Everything in space, with how strongly to draw it.
pub fn compose_space(
    rows: &[Row],
    hidden: &[bool],
    selected: Option<usize>,
    step: Option<usize>,
    progress: f64,
    frame: Frame,
) -> Vec<(Item, f64)> {
    let mut out: Vec<(Item, f64)> = Vec::new();
    out.extend(model::grid(2.0, 0.5).into_iter().map(|i| (i, 1.0)));
    out.extend(model::axes(1.3).into_iter().map(|i| (i, 1.0)));

    for (i, row) in rows.iter().enumerate() {
        if hidden.get(i).copied().unwrap_or(false) {
            continue;
        }
        if let Some(Ok(Value::Vec3(v))) = &row.value {
            out.push((
                Item::Arrow {
                    from: Vec3::ZERO,
                    to: *v,
                    ink: Ink::Row((i % PALETTE.len()) as u8),
                    width: 2.0,
                },
                if selected.is_some() && selected != Some(i) {
                    0.45
                } else {
                    1.0
                },
            ));
            if let Some(name) = &row.name {
                out.push((
                    Item::Label {
                        at: *v * 1.06,
                        text: name.clone(),
                        ink: Ink::Row((i % PALETTE.len()) as u8),
                    },
                    1.0,
                ));
            }
        }
    }

    let chosen = selected.and_then(|i| rows.get(i));
    let playing = chosen.and_then(|row| attitude_at(&turns(row), progress));
    if let Some(i) = attitude_row(rows, hidden, selected) {
        let row = &rows[i];
        let animated = (Some(i) == selected).then_some(playing).flatten();
        let at = animated.map(|(_, q)| q).or_else(|| {
            row.value
                .as_ref()
                .and_then(|v| v.as_ref().ok())
                .and_then(attitude_of)
        });
        if let Some(q) = at {
            out.extend(model::quad(q, frame, false).into_iter().map(|i| (i, 1.0)));
            out.extend(model::triad(q, 0.9, false).into_iter().map(|i| (i, 1.0)));
        }
        // Where the turn being played started, as a ghost.
        if let Some((k, _)) = animated
            && let Some((from, _)) = row.steps.get(k).and_then(|s| s.turn)
            && progress < 1.0
        {
            out.extend(model::quad(from, frame, true).into_iter().map(|i| (i, 1.0)));
        }
    }

    if let Some(row) = chosen
        && let Some(k) = active_step(
            row,
            step,
            playing.filter(|_| progress < 1.0).map(|(k, _)| k),
        )
    {
        out.extend(shapes(&row.steps[k].shapes));
    }
    out
}

/// Everything on the plane: its grid out to `half`, the rows' plane vectors
/// and the working of the selected row's step.
pub fn compose_plane(
    rows: &[Row],
    hidden: &[bool],
    selected: Option<usize>,
    step: Option<usize>,
    half: f64,
    grid_step: f64,
    centre: (f64, f64),
) -> Vec<(Item, f64)> {
    let mut out: Vec<(Item, f64)> = Vec::new();
    let from = |c: f64| ((c - half) / grid_step).floor() as i64;
    let to = |c: f64| ((c + half) / grid_step).ceil() as i64;
    for k in from(centre.0)..=to(centre.0) {
        let x = k as f64 * grid_step;
        out.push((
            Item::Line {
                a: Vec3::new(x, centre.1 - half, 0.0),
                b: Vec3::new(x, centre.1 + half, 0.0),
                ink: Ink::Grid,
                width: 0.6,
                dashed: false,
            },
            1.0,
        ));
    }
    for k in from(centre.1)..=to(centre.1) {
        let y = k as f64 * grid_step;
        out.push((
            Item::Line {
                a: Vec3::new(centre.0 - half, y, 0.0),
                b: Vec3::new(centre.0 + half, y, 0.0),
                ink: Ink::Grid,
                width: 0.6,
                dashed: false,
            },
            1.0,
        ));
    }
    for (a, b, ink, name) in [
        (
            Vec3::new(centre.0 - half, 0.0, 0.0),
            Vec3::new(centre.0 + half, 0.0, 0.0),
            Ink::AxisX,
            "X",
        ),
        (
            Vec3::new(0.0, centre.1 - half, 0.0),
            Vec3::new(0.0, centre.1 + half, 0.0),
            Ink::AxisY,
            "Y",
        ),
    ] {
        out.push((
            Item::Line {
                a,
                b,
                ink,
                width: 1.1,
                dashed: false,
            },
            1.0,
        ));
        out.push((
            Item::Label {
                at: b * 0.97,
                text: name.into(),
                ink,
            },
            1.0,
        ));
    }
    for (i, row) in rows.iter().enumerate() {
        if hidden.get(i).copied().unwrap_or(false) {
            continue;
        }
        if let Some(Ok(Value::Vec2(v))) = &row.value {
            let tip = Vec3::new(v.x, v.y, 0.0);
            out.push((
                Item::Arrow {
                    from: Vec3::ZERO,
                    to: tip,
                    ink: Ink::Row((i % PALETTE.len()) as u8),
                    width: 2.0,
                },
                if selected.is_some() && selected != Some(i) {
                    0.45
                } else {
                    1.0
                },
            ));
            if let Some(name) = &row.name {
                out.push((
                    Item::Label {
                        at: tip * 1.05,
                        text: name.clone(),
                        ink: Ink::Row((i % PALETTE.len()) as u8),
                    },
                    1.0,
                ));
            }
        }
    }
    if let Some(row) = selected.and_then(|i| rows.get(i))
        && let Some(k) = active_step(row, step, None)
    {
        out.extend(shapes(&row.steps[k].shapes));
    }
    out
}

fn shapes(shapes: &[Shape]) -> Vec<(Item, f64)> {
    shapes
        .iter()
        .flat_map(model::working)
        .map(|item| (item, 1.0))
        .collect()
}

/// Whether the plane is the view to show: the selected row is a vector in
/// the plane, or nothing is selected and the plane is all the sheet has.
pub fn plane_suits(rows: &[Row], selected: Option<usize>) -> bool {
    let flat = |row: &Row| matches!(row.value, Some(Ok(Value::Vec2(_))));
    let spatial = |row: &Row| {
        matches!(
            row.value,
            Some(Ok(Value::Vec3(_)
                | Value::Quat(_)
                | Value::Euler(_)
                | Value::Mat3(_)))
        )
    };
    match selected.and_then(|i| rows.get(i)) {
        Some(row) if flat(row) => true,
        Some(row) if spatial(row) => false,
        _ => rows.iter().any(flat) && !rows.iter().any(spatial),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::sheet::{Live, MathSheet, evaluate};

    fn rows(lines: &[&str]) -> Vec<Row> {
        evaluate(
            &MathSheet {
                frame: Frame::ZUp,
                rows: lines.iter().map(|l| l.to_string()).collect(),
            },
            &Live::default(),
        )
    }

    /// Playing `euler` is yaw, then pitch, then roll: a third of the way
    /// through, the yaw is done and nothing else has started.
    #[test]
    fn playing_an_euler_row_turns_through_its_three_steps_in_order() {
        let rows = rows(&["q = euler(30°, 20°, 90°)"]);
        let turns = turns(&rows[0]);
        assert_eq!(turns.len(), 3);
        let (step, q) = attitude_at(&turns, 1.0 / 3.0 + 1e-9).unwrap();
        assert_eq!(step, 1);
        assert!(q.angle_to(Quat::about_z(90f64.to_radians())) < 1e-6);
        let (_, end) = attitude_at(&turns, 1.0).unwrap();
        let Some(Ok(Value::Quat(value))) = rows[0].value else {
            panic!();
        };
        assert!(end.angle_to(value) < 1e-9);
        assert!(attitude_at(&[], 0.5).is_none());
    }

    /// The aircraft is the selected row's attitude, else the last one shown.
    #[test]
    fn the_aircraft_is_the_selected_attitude_or_the_last_one_shown() {
        let rows = rows(&[
            "a = euler(0, 0, 0)",
            "v = (1, 0, 0)",
            "b = euler(10°, 0, 0)",
        ]);
        assert_eq!(attitude_row(&rows, &[], None), Some(2));
        assert_eq!(attitude_row(&rows, &[], Some(0)), Some(0));
        assert_eq!(attitude_row(&rows, &[false, false, true], None), Some(0));
        assert_eq!(attitude_row(&rows, &[], Some(1)), Some(2));
    }

    #[test]
    fn the_plane_is_shown_for_plane_vectors_and_space_for_the_rest() {
        let plane = rows(&["a = (1, 2)", "b = (3, 4)"]);
        assert!(plane_suits(&plane, None));
        let mixed = rows(&["a = (1, 2)", "q = euler(0, 0, 0)"]);
        assert!(!plane_suits(&mixed, None));
        assert!(plane_suits(&mixed, Some(0)));
    }

    #[test]
    fn a_step_being_read_wins_over_the_last_one_with_something_to_draw() {
        let rows = rows(&["v = rotate(axis_angle(Z, 90°), X)"]);
        let row = &rows[0];
        assert_eq!(active_step(row, Some(0), None), Some(0));
        assert_eq!(active_step(row, None, None), Some(row.steps.len() - 1));
        assert_eq!(active_step(row, Some(99), None), Some(row.steps.len() - 1));
    }
}
