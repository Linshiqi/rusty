//! The view: space with the aircraft in it, or the plane — the rows' vectors
//! in their colours, the selected row's attitude as the aircraft, and the
//! working of the step being read drawn where it happens.

use leptos::{ev, html, prelude::*};

use rusty_embed::spatial::sheet::{Row, Shape, Step, Value};
use rusty_embed::spatial::{Frame, Quat, Vec3};
use rusty_i18n::t;

use super::PALETTE;
use crate::scene::{self, Camera, Ink, Item, Preset, Projector, Svg, model};
use crate::state::AppState;

/// The turns a row's steps make, in order: what playing it plays.
pub(super) fn turns(row: &Row) -> Vec<(usize, (Quat, Quat))> {
    row.steps
        .iter()
        .enumerate()
        .filter_map(|(i, step)| step.turn.map(|turn| (i, turn)))
        .collect()
}

/// Where the turns have got to at `progress` (0 to 1 over all of them), and
/// which step that is.
pub(super) fn attitude_at(turns: &[(usize, (Quat, Quat))], progress: f64) -> Option<(usize, Quat)> {
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
pub(super) fn active_step(
    row: &Row,
    chosen: Option<usize>,
    playing_at: Option<usize>,
) -> Option<usize> {
    chosen
        .filter(|i| *i < row.steps.len())
        .or(playing_at)
        .or_else(|| row.steps.iter().rposition(|s: &Step| !s.shapes.is_empty()))
}

/// The attitude the aircraft is drawn at: the selected row's when it has
/// one, else the last shown row that is one.
fn attitude_row(rows: &[Row], hidden: &[bool], selected: Option<usize>) -> Option<usize> {
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
pub(super) fn compose_space(
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
pub(super) fn compose_plane(
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
fn plane_suits(rows: &[Row], selected: Option<usize>) -> bool {
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

/// A colour for what something stands for: the theme's, through its
/// variables, except the axes and the rows, which are the same colours in
/// every theme for the reason the Git lanes are.
fn colour(ink: Ink) -> &'static str {
    match ink {
        Ink::AxisX => "#e5484d",
        Ink::AxisY => "#30a46c",
        Ink::AxisZ => "#3e63dd",
        Ink::Grid => "var(--line)",
        Ink::Body => "var(--label-2)",
        Ink::Front | Ink::Result => "var(--rust)",
        Ink::Rotor | Ink::Ghost | Ink::Guide => "var(--label-3)",
        Ink::First => "var(--slate)",
        Ink::Second => "var(--amber)",
        Ink::Term => "var(--patina)",
        Ink::Row(i) => PALETTE[usize::from(i) % PALETTE.len()],
    }
}

#[component]
pub fn SceneView(rows: Memo<Vec<Row>>) -> impl IntoView {
    let state = AppState::expect();
    let host = NodeRef::<html::Div>::new();
    let size = RwSignal::new((800.0_f64, 480.0_f64));

    // The view's size follows the panel's dividers, the dock opening under
    // it and the window — so an observer on the element, not the window's
    // resize, which the dock closing never fires. The callback is forgotten
    // rather than kept, and reads only through `try_`, as the Git log's is.
    let observer = StoredValue::new_local(None::<web_sys::ResizeObserver>);
    Effect::new(move |_| {
        use wasm_bindgen::{JsCast, closure::Closure};
        let Some(element) = host.get() else {
            return;
        };
        let read = move |el: &web_sys::HtmlDivElement| {
            let (w, h) = (f64::from(el.client_width()), f64::from(el.client_height()));
            if w > 0.0 && h > 0.0 && size.try_get_untracked() != Some((w, h)) {
                let _ = size.try_set((w, h));
            }
        };
        read(&element);
        let measure = Closure::<dyn FnMut()>::new(move || {
            if let Some(Some(el)) = host.try_get_untracked() {
                read(&el);
            }
        });
        if let Ok(watch) = web_sys::ResizeObserver::new(measure.as_ref().unchecked_ref()) {
            watch.observe(&element);
            observer.update_value(|slot| {
                if let Some(old) = slot.replace(watch) {
                    old.disconnect();
                }
            });
        }
        measure.forget();
    });
    on_cleanup(move || {
        observer.try_update_value(|slot| {
            if let Some(watch) = slot.take() {
                watch.disconnect();
            }
        });
    });

    // The plane's own pan and zoom: its centre, and a factor on the scale
    // that fits the vectors.
    let plane = RwSignal::new((0.0_f64, 0.0_f64, 1.0_f64));
    let drag: RwSignal<Option<(f64, f64)>> = RwSignal::new(None);

    let flat = Memo::new(move |_| {
        let chosen = state.math.flat.get();
        let selected = state.math.selected.get();
        chosen.unwrap_or_else(|| rows.with(|r| plane_suits(r, selected)))
    });

    // The plane's scale: the vectors fitted, times the user's zoom.
    let plane_scale = move || {
        let (w, h) = size.get();
        let extent = rows.with(|rows| {
            rows.iter()
                .filter_map(|r| match &r.value {
                    Some(Ok(Value::Vec2(v))) => Some(v.norm()),
                    _ => None,
                })
                .fold(1.0_f64, f64::max)
        });
        (w.min(h) * 0.42 / extent) * plane.get().2
    };

    let drawn = Memo::new(move |_| {
        let (w, h) = size.get();
        let frame = state.math.sheet.with(|s| s.frame);
        let hidden = state.math.hidden.get();
        let selected = state.math.selected.get();
        let step = state.math.step.get();
        let progress = state.math.progress.get();
        rows.with(|rows| {
            if flat.get() {
                let scale = plane_scale();
                let (cx, cy, _) = plane.get();
                let half = (w.max(h) / scale) * 0.6;
                let grid_step = nice_step(80.0 / scale);
                let items = compose_plane(rows, &hidden, selected, step, half, grid_step, (cx, cy));
                scene::render(&items, &Projector::plane(scale, (cx, cy), w, h))
            } else {
                let camera = state.math.camera.get();
                let items = compose_space(rows, &hidden, selected, step, progress, frame);
                scene::render(&items, &Projector::new(&camera, frame, w, h))
            }
        })
    });

    let on_down = move |event: ev::PointerEvent| {
        if event.button() != 0 {
            return;
        }
        drag.set(Some((
            f64::from(event.client_x()),
            f64::from(event.client_y()),
        )));
        if let Some(target) = event.current_target() {
            use wasm_bindgen::JsCast;
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                let _ = el.set_pointer_capture(event.pointer_id());
            }
        }
    };
    let on_move = move |event: ev::PointerEvent| {
        let Some((x0, y0)) = drag.get_untracked() else {
            return;
        };
        let (x, y) = (f64::from(event.client_x()), f64::from(event.client_y()));
        let (dx, dy) = (x - x0, y - y0);
        drag.set(Some((x, y)));
        if flat.get_untracked() {
            let scale = plane_scale();
            plane.update(|(cx, cy, _)| {
                *cx -= dx / scale;
                *cy += dy / scale;
            });
        } else {
            let frame = state.math.sheet.with_untracked(|s| s.frame);
            state.math.camera.update(|c| *c = c.orbit(dx, dy, frame));
        }
    };
    let on_up = move |_: ev::PointerEvent| drag.set(None);
    let on_wheel = move |event: ev::WheelEvent| {
        event.prevent_default();
        let factor = (event.delta_y() * 0.0015).exp();
        if flat.get_untracked() {
            plane.update(|(_, _, zoom)| *zoom = (*zoom / factor).clamp(0.05, 50.0));
        } else {
            state.math.camera.update(|c| *c = c.zoomed(factor));
        }
    };

    let preset = move |p: Preset| {
        let frame = state.math.sheet.with_untracked(|s| s.frame);
        let distance = state.math.camera.with_untracked(|c| c.distance);
        state.math.camera.set(Camera {
            distance,
            ..Camera::preset(p, frame)
        });
    };
    let fit = move || {
        if flat.get_untracked() {
            plane.set((0.0, 0.0, 1.0));
            return;
        }
        let frame = state.math.sheet.with_untracked(|s| s.frame);
        let hidden = state.math.hidden.get_untracked();
        let selected = state.math.selected.get_untracked();
        let step = state.math.step.get_untracked();
        let items =
            rows.with_untracked(|rows| compose_space(rows, &hidden, selected, step, 1.0, frame));
        let items: Vec<Item> = items
            .into_iter()
            .filter(|(item, _)| !matches!(item, Item::Line { ink: Ink::Grid, .. }))
            .map(|(item, _)| item)
            .collect();
        let extent = model::extent(&items).max(1.3);
        state.math.camera.update(|c| *c = c.fitting(extent));
    };

    let tool =
        "h-[24px] rounded-[6px] px-2 text-footnote text-label-2 hover:bg-sunken hover:text-label";
    view! {
        <div node_ref=host class="absolute inset-0 overflow-hidden bg-content">
            <svg
                class="block h-full w-full touch-none select-none"
                style=move || if drag.get().is_some() { "cursor: grabbing" } else { "cursor: grab" }
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_up
                on:wheel=on_wheel
            >
                {move || drawn.get().into_iter().map(svg_of).collect_view()}
            </svg>
            <div class="absolute top-2 right-2 flex items-center gap-1 rounded-[8px] bg-raised/90 p-0.5 ring-1 ring-line">
                <button
                    type="button"
                    class=tool
                    on:click=move |_| state.math.flat.set(Some(!flat.get_untracked()))
                    title=t!("math.view-toggle-hint")
                >
                    {move || if flat.get() { t!("math.view-3d") } else { t!("math.view-2d") }}
                </button>
                {move || {
                    (!flat.get())
                        .then(|| {
                            view! {
                                <button type="button" class=tool on:click=move |_| preset(Preset::Chase)>
                                    {t!("math.view-chase")}
                                </button>
                                <button type="button" class=tool on:click=move |_| preset(Preset::Top)>
                                    {t!("math.view-top")}
                                </button>
                                <button type="button" class=tool on:click=move |_| preset(Preset::Front)>
                                    {t!("math.view-front")}
                                </button>
                                <button type="button" class=tool on:click=move |_| preset(Preset::Side)>
                                    {t!("math.view-side")}
                                </button>
                            }
                        })
                }}
                <button type="button" class=tool on:click=move |_| fit() title=t!("math.view-fit-hint")>
                    {t!("math.view-fit")}
                </button>
            </div>
            <div class="pointer-events-none absolute bottom-2 left-3 flex items-center gap-2 font-mono text-footnote text-label-3">
                <span style="color: #e5484d">"X"</span>
                <span style="color: #30a46c">"Y"</span>
                <span style="color: #3e63dd">"Z"</span>
                <span>
                    {move || {
                        match state.math.sheet.with(|s| s.frame) {
                            Frame::ZUp => t!("math.frame-z-up"),
                            Frame::ZDown => t!("math.frame-z-down"),
                        }
                    }}
                </span>
            </div>
        </div>
    }
}

/// A grid spacing near `target` that reads: 1, 2 or 5 times a power of ten.
fn nice_step(target: f64) -> f64 {
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

fn svg_of(d: scene::Drawn) -> AnyView {
    let colour = colour(d.ink);
    match d.svg {
        Svg::Path(path) => view! {
            <path
                d=path
                fill="none"
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-dasharray=if d.dashed { "4 3" } else { "" }
                style=format!("stroke: {colour}; stroke-width: {}; opacity: {}", d.width, d.opacity)
            />
        }
        .into_any(),
        Svg::Polygon(points) => view! {
            <polygon
                points=points
                stroke-linejoin="round"
                style=format!(
                    "fill: {colour}; fill-opacity: {}; stroke: {colour}; stroke-width: 1; opacity: {}",
                    d.fill,
                    d.opacity
                )
            />
        }
        .into_any(),
        Svg::Text { x, y, text } => view! {
            <text
                x=x + 4.0
                y=y - 4.0
                font-size="11"
                style=format!("fill: {colour}; font-family: var(--font-mono, monospace); opacity: {}", d.opacity)
            >
                {text}
            </text>
        }
        .into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::spatial::sheet::{Live, MathSheet, evaluate};

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
