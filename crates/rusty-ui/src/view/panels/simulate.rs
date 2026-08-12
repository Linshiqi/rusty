//! Running firmware with no hardware on the desk — and wiring the desk up.
//!
//! This page is a project of its own, aimed at Wokwi-grade simulation:
//! code on one side, the living board on the other, a growing part
//! catalogue, and user-defined parts via `.rusty/parts/*.toml` so a device
//! rusty never heard of can still be drawn and driven. The serial protocol
//! (`[rusty:gpio]`, and friends to come) is the contract every part speaks.
//!
//! The page is a small board editor in the Wokwi shape: a component library
//! on the left, a canvas with the devkit on the right, a toolbar above.
//! LEDs are added from the library, dragged into place, given a pin and a
//! colour, and saved into the project's `.rusty/sim.toml` — a file diffed
//! and reviewed like any other. At run time each LED lights from the pin
//! levels the firmware reports over serial, and the caption says exactly
//! that: the QEMU peripheral models expose no GPIO readback to do better.

use leptos::{ev, prelude::*};

use rusty_embed::SimBoard;

use crate::{controller, state::AppState, view::components::Empty};

#[component]
pub fn Simulate() -> impl IntoView {
    let state = AppState::expect();

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            controller::load_sim_plan(state);
        }
    });

    move || {
        if !state.has_project() {
            return view! {
                <Empty
                    title="No project open"
                    detail="Open a firmware project to run it in the simulator."
                />
            }
            .into_any();
        }
        let Some(plan) = state.sim_plan.get() else {
            return view! {
                <p class="px-5 py-4 text-callout text-label-3">"Working out the plan…"</p>
            }
            .into_any();
        };

        if !plan.supported {
            let reason = plan.reason.unwrap_or_default();
            return view! {
                <div class="px-5 py-4">
                    <p class="max-w-[64ch] text-callout leading-relaxed text-label-2 select-text">
                        {reason}
                    </p>
                </div>
            }
            .into_any();
        }

        let missing = plan.missing.clone();
        let blocked = !missing.is_empty();
        let running = state.session_running;

        view! {
            <div class="flex min-h-0 flex-1 flex-col">
                {(!missing.is_empty())
                    .then(|| {
                        view! {
                            <div class="flex flex-col gap-2.5 border-b border-line bg-amber-fill px-4 py-3">
                                <p class="text-callout font-medium">
                                    "The simulator needs tools that are not installed."
                                </p>
                                {missing
                                    .iter()
                                    .map(|tool| {
                                        let name = tool.name.clone();
                                        let install_name = name.clone();
                                        let manual = tool.install.clone();
                                        let failed = {
                                            let name = name.clone();
                                            Signal::derive(move || {
                                                state
                                                    .sim_install_failed
                                                    .with(|f| f.contains(&name))
                                            })
                                        };
                                        view! {
                                            <div class="flex flex-col gap-1.5">
                                                <div class="flex items-center gap-2.5">
                                                    <span class="font-mono text-footnote">
                                                        {name.clone()}
                                                    </span>
                                                    <button
                                                        type="button"
                                                        disabled=move || running.get()
                                                        on:click=move |_| {
                                                            controller::install_sim_tool(
                                                                state,
                                                                install_name.clone(),
                                                            )
                                                        }
                                                        class="rounded-[6px] bg-rust px-2.5 py-0.5 text-footnote font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                                                    >
                                                        "Install"
                                                    </button>
                                                </div>
                                                {move || {
                                                    let manual = manual.clone();
                                                    failed
                                                        .get()
                                                        .then(|| {
                                                            view! {
                                                                <div class="flex flex-col gap-1">
                                                                    <span class="text-footnote text-label-2">
                                                                        "Automatic install failed — by hand:"
                                                                    </span>
                                                                    <code class="rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote select-text">
                                                                        {manual}
                                                                    </code>
                                                                </div>
                                                            }
                                                        })
                                                }}
                                            </div>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })}

                <BoardEditor
                    board=plan.board.clone().unwrap_or_else(|| SimBoard {
                        chip: "esp32".to_string(),
                        kit_x: None,
                        kit_y: None,
                        leds: Vec::new(),
                        buttons: Vec::new(),
                        rgbs: Vec::new(),
                        sevens: Vec::new(),
                        displays: Vec::new(),
                        pots: Vec::new(),
                    })
                    blocked=blocked
                    user_parts=plan.parts.clone()
                />


            </div>
        }
        .into_any()
    }
}

/// One thing on the canvas, whatever its kind — the editor edits these and
/// splits them back into the wire model on save.
#[derive(Clone, PartialEq)]
enum PartKind {
    Led { color: String },
    Button,
    Rgb,
    /// pins are segments a..g.
    Seven,
    /// Shows the `[rusty:disp]` channel; needs no pins.
    Display,
    /// A slider sending `P<pin>=<0..255>`.
    Pot,
}

impl PartKind {
    /// How many wires this part runs to the chip.
    fn wires(&self) -> usize {
        match self {
            PartKind::Rgb => 3,
            PartKind::Seven => 7,
            PartKind::Display => 0,
            _ => 1,
        }
    }

    /// Fixed body width, so wire anchors land on the body edge exactly.
    fn width(&self) -> f64 {
        match self {
            PartKind::Seven => 78.0,
            PartKind::Display => 140.0,
            PartKind::Pot => 130.0,
            _ => 110.0,
        }
    }
}

#[derive(Clone, PartialEq)]
struct EditPart {
    kind: PartKind,
    /// pins[0] for LEDs, buttons and pots; RGB uses three; seven uses all.
    pins: [u8; 7],
    label: String,
    x: f64,
    y: f64,
    /// User-drawn bends per wire slot; empty routes automatically.
    waypoints: [Vec<(f64, f64)>; 7],
}

fn pins3(a: u8, b: u8, c: u8) -> [u8; 7] {
    [a, b, c, 0, 0, 0, 0]
}

/// The file's flat route list, spread into per-slot waypoint arrays.
fn waypoints_of(routes: &[Vec<(f64, f64)>]) -> [Vec<(f64, f64)>; 7] {
    let mut out: [Vec<(f64, f64)>; 7] = Default::default();
    for (slot, route) in routes.iter().take(7).enumerate() {
        out[slot] = route.clone();
    }
    out
}

/// Back to the file shape: one route per wire slot, trailing empties kept so
/// slot indexes stay aligned, fully-empty lists collapsed to nothing.
fn routes_of(waypoints: &[Vec<(f64, f64)>; 7], wires: usize) -> Vec<Vec<(f64, f64)>> {
    let slice = &waypoints[..wires];
    if slice.iter().all(Vec::is_empty) {
        Vec::new()
    } else {
        slice.to_vec()
    }
}

fn parts_of(board: &SimBoard) -> Vec<EditPart> {
    let mut out = Vec::new();
    for (index, led) in board.leds.iter().enumerate() {
        out.push(EditPart {
            kind: PartKind::Led {
                color: led.color.clone(),
            },
            pins: pins3(led.pin, 0, 0),
            label: led.label.clone(),
            x: led.x.unwrap_or(60.0),
            y: led.y.unwrap_or(40.0 + index as f64 * 56.0),
            waypoints: waypoints_of(&led.routes),
        });
    }
    for button in &board.buttons {
        out.push(EditPart {
            kind: PartKind::Button,
            pins: pins3(button.pin, 0, 0),
            label: button.label.clone(),
            x: button.x.unwrap_or(60.0),
            y: button.y.unwrap_or(180.0),
            waypoints: waypoints_of(&button.routes),
        });
    }
    for rgb in &board.rgbs {
        out.push(EditPart {
            kind: PartKind::Rgb,
            pins: pins3(rgb.r, rgb.g, rgb.b),
            label: rgb.label.clone(),
            x: rgb.x.unwrap_or(60.0),
            y: rgb.y.unwrap_or(240.0),
            waypoints: waypoints_of(&rgb.routes),
        });
    }
    for seven in &board.sevens {
        out.push(EditPart {
            kind: PartKind::Seven,
            pins: seven.pins,
            label: seven.label.clone(),
            x: seven.x.unwrap_or(160.0),
            y: seven.y.unwrap_or(60.0),
            waypoints: waypoints_of(&seven.routes),
        });
    }
    for display in &board.displays {
        out.push(EditPart {
            kind: PartKind::Display,
            pins: [0; 7],
            label: display.label.clone(),
            x: display.x.unwrap_or(160.0),
            y: display.y.unwrap_or(160.0),
            waypoints: Default::default(),
        });
    }
    for pot in &board.pots {
        out.push(EditPart {
            kind: PartKind::Pot,
            pins: pins3(pot.pin, 0, 0),
            label: pot.label.clone(),
            x: pot.x.unwrap_or(60.0),
            y: pot.y.unwrap_or(280.0),
            waypoints: waypoints_of(&pot.routes),
        });
    }
    out
}

fn board_of(chip: &str, kit: (f64, f64), parts: &[EditPart]) -> SimBoard {
    let mut board = SimBoard {
        chip: chip.to_string(),
        kit_x: Some(kit.0),
        kit_y: Some(kit.1),
        leds: Vec::new(),
        buttons: Vec::new(),
        rgbs: Vec::new(),
        sevens: Vec::new(),
        displays: Vec::new(),
        pots: Vec::new(),
    };
    for part in parts {
        match &part.kind {
            PartKind::Led { color } => board.leds.push(rusty_embed::SimLed {
                pin: part.pins[0],
                color: color.clone(),
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 1),
            }),
            PartKind::Button => board.buttons.push(rusty_embed::SimButton {
                pin: part.pins[0],
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 1),
            }),
            PartKind::Rgb => board.rgbs.push(rusty_embed::SimRgb {
                r: part.pins[0],
                g: part.pins[1],
                b: part.pins[2],
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 3),
            }),
            PartKind::Seven => board.sevens.push(rusty_embed::SimSeven {
                pins: part.pins,
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 7),
            }),
            PartKind::Display => board.displays.push(rusty_embed::SimDisplay {
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
            }),
            PartKind::Pot => board.pots.push(rusty_embed::SimPot {
                pin: part.pins[0],
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 1),
            }),
        }
    }
    board
}

/// Colour classes for a single-hue lamp.
fn lamp_classes(color: &str, lit: bool) -> &'static str {
    match (color, lit) {
        ("green", true) => "bg-[#3ddc84] shadow-[0_0_12px_3px_rgba(61,220,132,0.55)]",
        ("green", false) => "bg-[#1d4a2f]",
        ("blue", true) => "bg-[#4aa8ff] shadow-[0_0_12px_3px_rgba(74,168,255,0.55)]",
        ("blue", false) => "bg-[#1d3350]",
        ("red", true) => "bg-[#ff5c5c] shadow-[0_0_12px_3px_rgba(255,92,92,0.55)]",
        ("red", false) => "bg-[#4a1d1d]",
        ("yellow", true) => "bg-[#ffd75c] shadow-[0_0_12px_3px_rgba(255,215,92,0.5)]",
        ("yellow", false) => "bg-[#4a3f1d]",
        (_, true) => "bg-label shadow-[0_0_12px_3px_rgba(255,255,255,0.4)]",
        (_, false) => "bg-line-strong",
    }
}

/// Additive mix for the RGB lens, from three channel levels.
fn rgb_style(r: bool, g: bool, b: bool) -> &'static str {
    match (r, g, b) {
        (false, false, false) => "background: #2a2d33",
        (true, false, false) => "background: #ff5c5c; box-shadow: 0 0 12px 3px rgba(255,92,92,0.55)",
        (false, true, false) => "background: #3ddc84; box-shadow: 0 0 12px 3px rgba(61,220,132,0.55)",
        (false, false, true) => "background: #4aa8ff; box-shadow: 0 0 12px 3px rgba(74,168,255,0.55)",
        (true, true, false) => "background: #ffd75c; box-shadow: 0 0 12px 3px rgba(255,215,92,0.55)",
        (true, false, true) => "background: #d97cff; box-shadow: 0 0 12px 3px rgba(217,124,255,0.55)",
        (false, true, true) => "background: #5ce8e8; box-shadow: 0 0 12px 3px rgba(92,232,232,0.55)",
        (true, true, true) => "background: #f4f4f4; box-shadow: 0 0 12px 3px rgba(255,255,255,0.5)",
    }
}

/// One editor state the undo stack holds: every part plus the kit position.
type Snapshot = (Vec<EditPart>, (f64, f64));

/// What the pointer is currently moving.
#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Part { index: usize, dx: f64, dy: f64 },
    Kit { dx: f64, dy: f64 },
    /// Panning the sheet: screen-space start of view translation.
    Pan { start_tx: f64, start_ty: f64, px: f64, py: f64 },
    /// Pulling a new connection out of a part's pin stub.
    Wire { part: usize, slot: usize },
    /// Moving one waypoint of an existing wire.
    Waypoint { part: usize, slot: usize, index: usize },
}

const SNAP: f64 = 8.0;
const KIT_W: f64 = 150.0;
const KIT_H: f64 = 230.0;
/// Pin rows per side of the schematic devkit.
const KIT_ROWS: usize = 15;
/// "This pin is not wired to anything" — new parts start here, and
/// disconnecting returns here. 255 is no GPIO on any supported chip.
const UNWIRED: u8 = 255;

fn snap(value: f64) -> f64 {
    (value / SNAP).round() * SNAP
}

/// The classic 30-pin ESP32 devkit pinout, top to bottom, left then right.
/// `None` rows (power, ground, EN) refuse wires — an LED soldered to GND on
/// both ends is a diagram nobody meant.
fn kit_rows() -> [(&'static str, Option<u8>); 30] {
    [
        ("EN", None),
        ("36", Some(36)),
        ("39", Some(39)),
        ("34", Some(34)),
        ("35", Some(35)),
        ("32", Some(32)),
        ("33", Some(33)),
        ("25", Some(25)),
        ("26", Some(26)),
        ("27", Some(27)),
        ("14", Some(14)),
        ("12", Some(12)),
        ("13", Some(13)),
        ("GND", None),
        ("VIN", None),
        ("3V3", None),
        ("GND", None),
        ("15", Some(15)),
        ("2", Some(2)),
        ("4", Some(4)),
        ("16", Some(16)),
        ("17", Some(17)),
        ("5", Some(5)),
        ("18", Some(18)),
        ("19", Some(19)),
        ("21", Some(21)),
        ("RX", Some(3)),
        ("TX", Some(1)),
        ("22", Some(22)),
        ("23", Some(23)),
    ]
}

fn row_of_gpio(pin: u8) -> Option<usize> {
    kit_rows().iter().position(|(_, gpio)| *gpio == Some(pin))
}

/// World coordinates of a kit row's pin circle.
fn row_point(kit: (f64, f64), row: usize) -> (f64, f64) {
    if row < KIT_ROWS {
        (kit.0 + 10.0, kit.1 + 16.0 + row as f64 * 14.0)
    } else {
        (
            kit.0 + KIT_W - 10.0,
            kit.1 + 16.0 + (row - KIT_ROWS) as f64 * 14.0,
        )
    }
}

/// Which kit row a world point lands on, if any.
fn row_under(kit: (f64, f64), point: (f64, f64)) -> Option<usize> {
    let (kx, ky) = kit;
    let (x, y) = point;
    if y < ky + 9.0 || y > ky + 16.0 + KIT_ROWS as f64 * 14.0 {
        return None;
    }
    let row = (((y - ky - 16.0) / 14.0).round().max(0.0)) as usize;
    if row >= KIT_ROWS {
        return None;
    }
    if x >= kx - 8.0 && x <= kx + 30.0 {
        Some(row)
    } else if x >= kx + KIT_W - 30.0 && x <= kx + KIT_W + 8.0 {
        Some(row + KIT_ROWS)
    } else {
        None
    }
}

/// Where a part's `slot`-th wire leaves its body.
fn stub_point(part: &EditPart, slot: usize) -> (f64, f64) {
    (
        part.x + part.kind.width(),
        part.y + 14.0 + slot as f64 * 6.0,
    )
}

/// Index at which a clicked point should insert into a polyline's waypoint
/// list: after the nearest segment start.
fn insert_index(points: &[(f64, f64)], click: (f64, f64)) -> usize {
    let mut best = 0usize;
    let mut best_d = f64::MAX;
    for (i, pair) in points.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let (px, py) = (b.0 - a.0, b.1 - a.1);
        let len2 = (px * px + py * py).max(1e-6);
        let t = (((click.0 - a.0) * px + (click.1 - a.1) * py) / len2).clamp(0.0, 1.0);
        let (cx, cy) = (a.0 + t * px, a.1 + t * py);
        let d = (click.0 - cx).powi(2) + (click.1 - cy).powi(2);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// The label a single-pin part wears for its wiring state.
fn single_pin_label(kind: &PartKind, pin: u8) -> String {
    let base = match kind {
        PartKind::Button => "BTN",
        PartKind::Pot => "POT",
        _ => "GPIO",
    };
    if pin == UNWIRED {
        format!("{base} —")
    } else {
        format!("{base}{pin}")
    }
}

/// The editor: library, canvas, toolbar. Local state until Save writes it
/// into `.rusty/sim.toml` and the plan reloads.
#[component]
fn BoardEditor(
    board: SimBoard,
    blocked: bool,
    user_parts: Vec<rusty_embed::PartDef>,
) -> impl IntoView {
    let state = AppState::expect();
    let running = state.session_running;
    let chip = board.chip.clone();
    let chip_label = board.chip.to_uppercase();

    let parts = RwSignal::new(parts_of(&board));
    let kit_pos = RwSignal::new((board.kit_x.unwrap_or(460.0), board.kit_y.unwrap_or(40.0)));
    let dirty = RwSignal::new(false);
    let selected = RwSignal::new(None::<usize>);
    let selected_wire = RwSignal::new(None::<(usize, usize)>);
    let drag = RwSignal::new(None::<Drag>);
    // While pulling a wire: current cursor in world coords, and the row the
    // cursor hovers, when it is one that accepts wires.
    let ghost = RwSignal::new(None::<(f64, f64)>);
    let hover_row = RwSignal::new(None::<usize>);
    let view = RwSignal::new((0.0f64, 0.0f64, 1.0f64));
    let canvas: NodeRef<leptos::html::Div> = NodeRef::new();

    let history = RwSignal::new(Vec::<Snapshot>::new());
    let future = RwSignal::new(Vec::<Snapshot>::new());
    let checkpoint = move || {
        history.update(|h| {
            h.push((parts.get_untracked(), kit_pos.get_untracked()));
            if h.len() > 100 {
                h.remove(0);
            }
        });
        future.set(Vec::new());
    };
    let undo = move || {
        let Some((p, k)) = history.try_update(|h| h.pop()).flatten() else {
            return;
        };
        future.update(|f| f.push((parts.get_untracked(), kit_pos.get_untracked())));
        parts.set(p);
        kit_pos.set(k);
        dirty.set(true);
    };
    let redo = move || {
        let Some((p, k)) = future.try_update(|f| f.pop()).flatten() else {
            return;
        };
        history.update(|h| h.push((parts.get_untracked(), kit_pos.get_untracked())));
        parts.set(p);
        kit_pos.set(k);
        dirty.set(true);
    };

    // A new part arrives unwired: connecting it is the user's move, made by
    // pulling its stub to a chip pin. Auto-wiring guessed; this asks.
    let add_part = move |kind: PartKind, label_stub: String| {
        checkpoint();
        parts.update(|list| {
            let label = match &kind {
                PartKind::Led { .. } => "GPIO —".to_string(),
                PartKind::Button => "BTN —".to_string(),
                PartKind::Rgb => {
                    if label_stub.is_empty() {
                        "RGB".to_string()
                    } else {
                        label_stub.clone()
                    }
                }
                PartKind::Seven => "7SEG".to_string(),
                PartKind::Display => "DISPLAY".to_string(),
                PartKind::Pot => "POT —".to_string(),
            };
            list.push(EditPart {
                kind,
                pins: [UNWIRED; 7],
                label,
                x: snap(60.0 + (list.len() as f64 * 16.0) % 90.0),
                y: snap(40.0 + (list.len() as f64 * 52.0) % 240.0),
                waypoints: Default::default(),
            });
            selected.set(Some(list.len() - 1));
        });
        dirty.set(true);
    };

    let save = move |_| {
        let board = board_of(&chip, kit_pos.get_untracked(), &parts.get_untracked());
        controller::save_sim_board(state, board, dirty);
    };

    let to_world = move |client_x: f64, client_y: f64| -> (f64, f64) {
        let Some(element) = canvas.get_untracked() else {
            return (client_x, client_y);
        };
        let rect = element.get_bounding_client_rect();
        let (tx, ty, k) = view.get_untracked();
        (
            (client_x - rect.left() - tx) / k,
            (client_y - rect.top() - ty) / k,
        )
    };

    // Disconnect the selected wire, or remove the selected part.
    let delete_selection = move || {
        if let Some((part, slot)) = selected_wire.get_untracked() {
            checkpoint();
            parts.update(|list| {
                if let Some(p) = list.get_mut(part) {
                    p.pins[slot] = UNWIRED;
                    p.waypoints[slot].clear();
                    if p.kind.wires() == 1 {
                        p.label = single_pin_label(&p.kind, UNWIRED);
                    }
                }
            });
            selected_wire.set(None);
            dirty.set(true);
        } else if let Some(index) = selected.get_untracked() {
            checkpoint();
            parts.update(|list| {
                if index < list.len() {
                    list.remove(index);
                }
            });
            selected.set(None);
            dirty.set(true);
        }
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex flex-none items-center gap-1.5 border-b border-line px-4 py-2">
                {move || {
                    if running.get() {
                        view! {
                            <button
                                type="button"
                                on:click=move |_| controller::stop_session_now(state)
                                class="rounded-[7px] bg-crimson px-4 py-1.5 text-callout font-medium text-white hover:opacity-90"
                            >
                                "Stop"
                            </button>
                        }
                            .into_any()
                    } else {
                        let disabled = blocked;
                        view! {
                            <button
                                type="button"
                                disabled=disabled
                                on:click=move |_| controller::run_simulation(state)
                                class="rounded-[7px] bg-rust px-4 py-1.5 text-callout font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                            >
                                "Build and simulate"
                            </button>
                        }
                            .into_any()
                    }
                }}
                <button
                    type="button"
                    disabled=move || !dirty.get()
                    on:click=save
                    class="rounded-[7px] px-3 py-1.5 text-callout text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-35"
                >
                    "Save layout"
                </button>
                <span class="mx-1 h-5 w-px bg-line" />
                <button
                    type="button"
                    title="Undo (Ctrl+Z)"
                    disabled=move || history.with(Vec::is_empty)
                    on:click=move |_| undo()
                    class="grid size-7 place-items-center rounded-[6px] text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-35"
                >
                    "↶"
                </button>
                <button
                    type="button"
                    title="Redo (Ctrl+Y)"
                    disabled=move || future.with(Vec::is_empty)
                    on:click=move |_| redo()
                    class="grid size-7 place-items-center rounded-[6px] text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-35"
                >
                    "↷"
                </button>
                <span class="mx-1 h-5 w-px bg-line" />
                <button
                    type="button"
                    title="Zoom out"
                    on:click=move |_| {
                        view.update(|(_, _, k)| *k = (*k / 1.2).max(0.35))
                    }
                    class="grid size-7 place-items-center rounded-[6px] text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label"
                >
                    "−"
                </button>
                <span class="min-w-[5ch] text-center font-mono text-footnote text-label-3">
                    {move || format!("{:.0}%", view.get().2 * 100.0)}
                </span>
                <button
                    type="button"
                    title="Zoom in"
                    on:click=move |_| {
                        view.update(|(_, _, k)| *k = (*k * 1.2).min(2.5))
                    }
                    class="grid size-7 place-items-center rounded-[6px] text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label"
                >
                    "+"
                </button>
                <button
                    type="button"
                    title="Reset view"
                    on:click=move |_| view.set((0.0, 0.0, 1.0))
                    class="rounded-[6px] px-2 py-1 text-footnote text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label"
                >
                    "1:1"
                </button>
                {move || {
                    running
                        .get()
                        .then(|| {
                            view! {
                                <span class="text-footnote text-label-3">
                                    "running — buttons are live"
                                </span>
                            }
                        })
                }}
                <span class="flex-1" />
                <span class="text-caption text-label-4">
                    "wire a part by dragging its gold stub to a chip pin"
                </span>
            </div>

            <div class="flex min-h-0 flex-1">
                <div class="flex w-[160px] flex-none flex-col gap-1 overflow-y-auto border-r border-line bg-sidebar p-2">
                    <span class="px-1 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        "Parts"
                    </span>
                    {[
                        ("green", "bg-[#3ddc84]"),
                        ("blue", "bg-[#4aa8ff]"),
                        ("red", "bg-[#ff5c5c]"),
                        ("yellow", "bg-[#ffd75c]"),
                    ]
                        .into_iter()
                        .map(|(color, swatch)| {
                            view! {
                                <button
                                    type="button"
                                    title=format!("Add a {color} LED (unwired)")
                                    on:click=move |_| add_part(
                                        PartKind::Led {
                                            color: color.to_string(),
                                        },
                                        String::new(),
                                    )
                                    class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                                >
                                    <span class=format!("size-3.5 rounded-full {swatch}") />
                                    <span>{format!("{color} LED")}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                    <button
                        type="button"
                        title="Pressing it sends B<pin>=1/0 into the firmware's UART"
                        on:click=move |_| add_part(PartKind::Button, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="grid size-3.5 place-items-center rounded-[4px] bg-line-strong">
                            <span class="size-1.5 rounded-full bg-label-3" />
                        </span>
                        <span>"button"</span>
                    </button>
                    <button
                        type="button"
                        title="Three pins, additive colour"
                        on:click=move |_| add_part(PartKind::Rgb, "RGB".to_string())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="size-3.5 rounded-full bg-[conic-gradient(#ff5c5c,#3ddc84,#4aa8ff,#ff5c5c)]" />
                        <span>"RGB LED"</span>
                    </button>
                    <button
                        type="button"
                        title="Seven segments, one GPIO each"
                        on:click=move |_| add_part(PartKind::Seven, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="grid size-3.5 place-items-center rounded-[3px] bg-[#3a2323] font-mono text-[9px] leading-none text-[#ff5c5c]">
                            "8"
                        </span>
                        <span>"7-segment"</span>
                    </button>
                    <button
                        type="button"
                        title="A text screen fed by [rusty:disp] serial lines"
                        on:click=move |_| add_part(PartKind::Display, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="h-3 w-4 rounded-[2px] bg-[#0d1a12] ring-1 ring-[#1d4a2f]" />
                        <span>"display"</span>
                    </button>
                    <button
                        type="button"
                        title="A slider that sends P<pin>=<0..255>"
                        on:click=move |_| add_part(PartKind::Pot, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="grid size-3.5 place-items-center rounded-full bg-line-strong">
                            <span class="h-2 w-0.5 bg-[#c9a227]" />
                        </span>
                        <span>"potentiometer"</span>
                    </button>
                    {(!user_parts.is_empty())
                        .then(|| {
                            view! {
                                <span class="mt-2 px-1 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                    "Custom"
                                </span>
                            }
                        })}
                    {user_parts
                        .iter()
                        .map(|def| {
                            let name = def.name.clone();
                            let color = def.color.clone();
                            let add_color = color.clone();
                            let add_name = name.clone();
                            view! {
                                <button
                                    type="button"
                                    title=format!("{name} — from .rusty/parts/")
                                    on:click=move |_| {
                                        add_part(
                                            PartKind::Led {
                                                color: add_color.clone(),
                                            },
                                            add_name.clone(),
                                        )
                                    }
                                    class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                                >
                                    <span class=format!(
                                        "size-3.5 rounded-full {}",
                                        match color.as_str() {
                                            "blue" => "bg-[#4aa8ff]",
                                            "red" => "bg-[#ff5c5c]",
                                            "yellow" => "bg-[#ffd75c]",
                                            _ => "bg-[#3ddc84]",
                                        },
                                    ) />
                                    <span>{name.clone()}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                    <p class="mt-1 px-1 text-caption leading-snug text-label-4">
                        "your own parts: .rusty/parts/*.toml"
                    </p>
                </div>

                <div
                    node_ref=canvas
                    tabindex="0"
                    on:keydown=move |event: ev::KeyboardEvent| {
                        match event.key().as_str() {
                            "Delete" | "Backspace" => {
                                event.prevent_default();
                                delete_selection();
                            }
                            "Escape" => {
                                selected_wire.set(None);
                                selected.set(None);
                            }
                            _ if event.ctrl_key()
                                && event.key().eq_ignore_ascii_case("z") => {
                                event.prevent_default();
                                if event.shift_key() { redo() } else { undo() }
                            }
                            _ if event.ctrl_key()
                                && event.key().eq_ignore_ascii_case("y") => {
                                event.prevent_default();
                                redo();
                            }
                            _ => {}
                        }
                    }
                    on:wheel=move |event: ev::WheelEvent| {
                        event.prevent_default();
                        let Some(element) = canvas.get_untracked() else {
                            return;
                        };
                        let rect = element.get_bounding_client_rect();
                        let cx = f64::from(event.client_x()) - rect.left();
                        let cy = f64::from(event.client_y()) - rect.top();
                        view.update(|(tx, ty, k)| {
                            let factor = if event.delta_y() < 0.0 { 1.12 } else { 1.0 / 1.12 };
                            let next = (*k * factor).clamp(0.35, 2.5);
                            let real = next / *k;
                            *tx = cx - (cx - *tx) * real;
                            *ty = cy - (cy - *ty) * real;
                            *k = next;
                        });
                    }
                    on:pointerdown=move |event: ev::PointerEvent| {
                        if let Some(element) = canvas.get_untracked()
                            && let Some(target) = event.target()
                            && let Ok(node) = wasm_bindgen::JsCast::dyn_into::<web_sys::Node>(target)
                            && (element.is_same_node(Some(&node))
                                || node.node_name() == "svg"
                                || node.node_name() == "rect")
                        {
                            let _ = element.focus();
                            let (tx, ty, _) = view.get_untracked();
                            drag.set(Some(Drag::Pan {
                                start_tx: tx,
                                start_ty: ty,
                                px: f64::from(event.client_x()),
                                py: f64::from(event.client_y()),
                            }));
                            selected.set(None);
                            selected_wire.set(None);
                        }
                    }
                    on:pointermove=move |event: ev::PointerEvent| {
                        let Some(current) = drag.get_untracked() else {
                            return;
                        };
                        let world = to_world(
                            f64::from(event.client_x()),
                            f64::from(event.client_y()),
                        );
                        match current {
                            Drag::Pan { start_tx, start_ty, px, py } => {
                                view.update(|(tx, ty, _)| {
                                    *tx = start_tx + f64::from(event.client_x()) - px;
                                    *ty = start_ty + f64::from(event.client_y()) - py;
                                });
                            }
                            Drag::Part { index, dx, dy } => {
                                parts.update(|list| {
                                    if let Some(part) = list.get_mut(index) {
                                        part.x = snap(world.0 - dx);
                                        part.y = snap(world.1 - dy);
                                    }
                                });
                                dirty.set(true);
                            }
                            Drag::Kit { dx, dy } => {
                                kit_pos.set((snap(world.0 - dx), snap(world.1 - dy)));
                                dirty.set(true);
                            }
                            Drag::Wire { .. } => {
                                ghost.set(Some(world));
                                let row = row_under(kit_pos.get_untracked(), world)
                                    .filter(|r| kit_rows()[*r].1.is_some());
                                hover_row.set(row);
                            }
                            Drag::Waypoint { part, slot, index } => {
                                parts.update(|list| {
                                    if let Some(p) = list.get_mut(part)
                                        && let Some(wp) = p.waypoints[slot].get_mut(index)
                                    {
                                        *wp = (snap(world.0), snap(world.1));
                                    }
                                });
                                dirty.set(true);
                            }
                        }
                    }
                    on:pointerup=move |_| {
                        if let Some(Drag::Wire { part, slot }) = drag.get_untracked() {
                            // Landing on a GPIO row wires the pin; anywhere
                            // else cancels. Wiring IS pin assignment.
                            if let Some(row) = hover_row.get_untracked()
                                && let Some(gpio) = kit_rows()[row].1
                            {
                                checkpoint();
                                parts.update(|list| {
                                    if let Some(p) = list.get_mut(part) {
                                        p.pins[slot] = gpio;
                                        p.waypoints[slot].clear();
                                        if p.kind.wires() == 1 {
                                            p.label = single_pin_label(&p.kind, gpio);
                                        }
                                    }
                                });
                                selected_wire.set(Some((part, slot)));
                                dirty.set(true);
                            }
                        }
                        ghost.set(None);
                        hover_row.set(None);
                        drag.set(None);
                    }
                    on:pointerleave=move |_| {
                        ghost.set(None);
                        hover_row.set(None);
                        drag.set(None);
                    }
                    class="relative min-w-0 flex-1 overflow-hidden bg-[#101216] outline-none"
                >
                    <div
                        class="absolute"
                        style=move || {
                            let (tx, ty, k) = view.get();
                            format!(
                                "transform: translate({tx}px, {ty}px) scale({k}); transform-origin: 0 0",
                            )
                        }
                    >
                        <svg
                            class="pointer-events-none absolute"
                            style="left: -2000px; top: -2000px"
                            width="6000"
                            height="6000"
                        >
                            <defs>
                                <pattern
                                    id="sheet-grid"
                                    width="16"
                                    height="16"
                                    patternUnits="userSpaceOnUse"
                                >
                                    <circle cx="1" cy="1" r="1" fill="#23262c" />
                                </pattern>
                            </defs>
                            <rect width="6000" height="6000" fill="url(#sheet-grid)" />
                            <g transform="translate(2000, 2000)">
                                // ── wires: one per connected pin ────────────
                                {move || {
                                    let kit = kit_pos.get();
                                    let picked = selected_wire.get();
                                    parts
                                        .get()
                                        .iter()
                                        .enumerate()
                                        .flat_map(|(part_index, part)| {
                                            (0..part.kind.wires())
                                                .filter_map(|slot| {
                                                    let pin = part.pins[slot];
                                                    if pin == UNWIRED {
                                                        return None;
                                                    }
                                                    let row = row_of_gpio(pin)?;
                                                    let from = stub_point(part, slot);
                                                    let to = row_point(kit, row);
                                                    let mut points = vec![from];
                                                    points
                                                        .extend(part.waypoints[slot].iter().copied());
                                                    // Without user waypoints,
                                                    // an L keeps it tidy.
                                                    if part.waypoints[slot].is_empty() {
                                                        let mid = if row < KIT_ROWS {
                                                            to.0 - 24.0 - (row as f64 * 4.0)
                                                        } else {
                                                            to.0 + 24.0
                                                                + ((row - KIT_ROWS) as f64 * 4.0)
                                                        };
                                                        points.push((mid, from.1));
                                                        points.push((mid, to.1));
                                                    }
                                                    points.push(to);
                                                    let path = points
                                                        .iter()
                                                        .map(|(x, y)| format!("{x},{y}"))
                                                        .collect::<Vec<_>>()
                                                        .join(" ");
                                                    let is_picked =
                                                        picked == Some((part_index, slot));
                                                    let stroke = if is_picked {
                                                        "#e05d38"
                                                    } else {
                                                        "#7d8694"
                                                    };
                                                    let width = if is_picked { "2.4" } else { "1.6" };
                                                    let handles = is_picked
                                                        .then(|| {
                                                            parts
                                                                .with_untracked(|l| {
                                                                    l[part_index].waypoints[slot]
                                                                        .clone()
                                                                })
                                                                .into_iter()
                                                                .enumerate()
                                                                .map(|(wp_index, (wx, wy))| {
                                                                    view! {
                                                                        <rect
                                                                            x=wx - 3.5
                                                                            y=wy - 3.5
                                                                            width="7"
                                                                            height="7"
                                                                            fill="#c9a227"
                                                                            stroke="#101216"
                                                                            style="pointer-events: auto; cursor: move"
                                                                            on:pointerdown=move |event: ev::PointerEvent| {
                                                                                event.stop_propagation();
                                                                                checkpoint();
                                                                                drag.set(Some(Drag::Waypoint {
                                                                                    part: part_index,
                                                                                    slot,
                                                                                    index: wp_index,
                                                                                }));
                                                                            }
                                                                        />
                                                                    }
                                                                })
                                                                .collect_view()
                                                        });
                                                    Some(view! {
                                                        // Fat invisible twin
                                                        // makes the wire
                                                        // clickable.
                                                        <polyline
                                                            points=path.clone()
                                                            fill="none"
                                                            stroke="transparent"
                                                            stroke-width="10"
                                                            style="pointer-events: stroke; cursor: pointer"
                                                            on:pointerdown=move |event: ev::PointerEvent| {
                                                                event.stop_propagation();
                                                            }
                                                            on:click=move |event: ev::MouseEvent| {
                                                                event.stop_propagation();
                                                                selected.set(None);
                                                                selected_wire
                                                                    .set(Some((part_index, slot)));
                                                                if let Some(element) =
                                                                    canvas.get_untracked()
                                                                {
                                                                    let _ = element.focus();
                                                                }
                                                            }
                                                            on:dblclick=move |event: ev::MouseEvent| {
                                                                event.stop_propagation();
                                                                let world = to_world(
                                                                    f64::from(event.client_x()),
                                                                    f64::from(event.client_y()),
                                                                );
                                                                checkpoint();
                                                                parts.update(|list| {
                                                                    let Some(p) =
                                                                        list.get_mut(part_index)
                                                                    else {
                                                                        return;
                                                                    };
                                                                    // Rebuild the drawn point list
                                                                    // to find where this belongs.
                                                                    let from =
                                                                        stub_point(p, slot);
                                                                    let row = row_of_gpio(p.pins[slot]);
                                                                    let mut pts = vec![from];
                                                                    pts.extend(
                                                                        p.waypoints[slot]
                                                                            .iter()
                                                                            .copied(),
                                                                    );
                                                                    if let Some(row) = row {
                                                                        pts.push(row_point(
                                                                            kit_pos.get_untracked(),
                                                                            row,
                                                                        ));
                                                                    }
                                                                    let at =
                                                                        insert_index(&pts, world);
                                                                    p.waypoints[slot].insert(
                                                                        at.min(
                                                                            p.waypoints[slot].len(),
                                                                        ),
                                                                        (
                                                                            snap(world.0),
                                                                            snap(world.1),
                                                                        ),
                                                                    );
                                                                });
                                                                selected_wire
                                                                    .set(Some((part_index, slot)));
                                                                dirty.set(true);
                                                            }
                                                        />
                                                        <polyline
                                                            points=path
                                                            fill="none"
                                                            stroke=stroke
                                                            stroke-width=width
                                                        />
                                                        <circle cx=from.0 cy=from.1 r="2.6" fill="#c9a227" />
                                                        <circle cx=to.0 cy=to.1 r="2.6" fill="#c9a227" />
                                                        {handles}
                                                    })
                                                })
                                                .collect::<Vec<_>>()
                                        })
                                        .collect_view()
                                }}

                                // ── the ghost while pulling a new wire ──────
                                {move || {
                                    let target = ghost.get()?;
                                    let Some(Drag::Wire { part, slot }) = drag.get() else {
                                        return None;
                                    };
                                    let from = parts
                                        .with(|list| list.get(part).map(|p| stub_point(p, slot)))?;
                                    Some(view! {
                                        <line
                                            x1=from.0
                                            y1=from.1
                                            x2=target.0
                                            y2=target.1
                                            stroke="#e0a838"
                                            stroke-width="1.8"
                                            stroke-dasharray="5 4"
                                        />
                                    })
                                }}
                            </g>
                        </svg>

                        // the devkit — labelled pins, drop targets, draggable
                        {move || {
                            let (kx, ky) = kit_pos.get();
                            let hovered = hover_row.get();
                            view! {
                                <div
                                    on:pointerdown=move |event: ev::PointerEvent| {
                                        event.prevent_default();
                                        event.stop_propagation();
                                        checkpoint();
                                        let world = to_world(
                                            f64::from(event.client_x()),
                                            f64::from(event.client_y()),
                                        );
                                        let (kx, ky) = kit_pos.get_untracked();
                                        drag.set(Some(Drag::Kit {
                                            dx: world.0 - kx,
                                            dy: world.1 - ky,
                                        }));
                                    }
                                    class="absolute cursor-grab"
                                    style=format!("left: {kx}px; top: {ky}px")
                                >
                                    <svg
                                        width=KIT_W
                                        height=KIT_H
                                        viewBox=format!("0 0 {KIT_W} {KIT_H}")
                                    >
                                        <rect x="4" y="2" width=KIT_W - 8.0 height=KIT_H - 4.0 rx="10" fill="#1a1d23" stroke="#454b56" stroke-width="1.5" />
                                        {kit_rows()
                                            .into_iter()
                                            .enumerate()
                                            .map(|(row, (name, gpio))| {
                                                let left = row < KIT_ROWS;
                                                let y = 16 + (row % KIT_ROWS) as i32 * 14;
                                                let cx = if left { 10.0 } else { KIT_W - 10.0 };
                                                let hot = hovered == Some(row);
                                                let fill = if hot {
                                                    "#e0a838"
                                                } else if gpio.is_some() {
                                                    "#c9a227"
                                                } else {
                                                    "#5a5142"
                                                };
                                                let r = if hot { 5.0 } else { 3.2 };
                                                view! {
                                                    <circle cx=cx cy=y r=r fill=fill />
                                                    <text
                                                        x=if left { 18.0 } else { KIT_W - 18.0 }
                                                        y=y + 3
                                                        text-anchor=if left { "start" } else { "end" }
                                                        font-family="ui-monospace"
                                                        font-size="7.5"
                                                        fill="#98a1ae"
                                                    >
                                                        {name}
                                                    </text>
                                                }
                                            })
                                            .collect_view()}
                                        <rect x="42" y="12" width=KIT_W - 84.0 height="84" rx="4" fill="#2e333b" stroke="#4a515d" />
                                        <text x=KIT_W / 2.0 y="58" text-anchor="middle" font-family="ui-monospace" font-size="12" fill="#aab3c0">
                                            {chip_label.clone()}
                                        </text>
                                        <rect x=KIT_W / 2.0 - 15.0 y=KIT_H - 22.0 width="30" height="14" rx="2" fill="#3a3e46" />
                                    </svg>
                                </div>
                            }
                        }}

                        {move || {
                            parts
                                .get()
                                .iter()
                                .enumerate()
                                .map(|(index, part)| {
                                    let x = part.x;
                                    let y = part.y;
                                    let width = part.kind.width();
                                    let label = part.label.clone();
                                    let kind = part.kind.clone();
                                    let grab_kind = part.kind.clone();
                                    let pins = part.pins;
                                    let wires = part.kind.wires();
                                    let is_selected =
                                        Signal::derive(move || selected.get() == Some(index));
                                    let level = move |pin: u8| {
                                        state
                                            .sim_gpio
                                            .with(|gpio| {
                                                gpio.get(&pin).copied().unwrap_or(false)
                                            })
                                    };

                                    let face = match kind.clone() {
                                        PartKind::Led { color } => view! {
                                            <span class=move || {
                                                format!(
                                                    "size-4 shrink-0 rounded-full transition-all duration-150 {}",
                                                    lamp_classes(&color, level(pins[0])),
                                                )
                                            } />
                                        }
                                            .into_any(),
                                        PartKind::Rgb => view! {
                                            <span
                                                class="size-4 shrink-0 rounded-full transition-all duration-150"
                                                style=move || rgb_style(
                                                    level(pins[0]),
                                                    level(pins[1]),
                                                    level(pins[2]),
                                                )
                                            />
                                        }
                                            .into_any(),
                                        PartKind::Seven => view! {
                                            <svg width="26" height="42" viewBox="0 0 26 42">
                                                {
                                                    let seg = move |slot: usize| {
                                                        if level(pins[slot]) {
                                                            "#ff5c5c"
                                                        } else {
                                                            "#3a2323"
                                                        }
                                                    };
                                                    view! {
                                                        <rect x="6" y="2" width="14" height="4" rx="2" fill=move || seg(0) />
                                                        <rect x="19" y="5" width="4" height="13" rx="2" fill=move || seg(1) />
                                                        <rect x="19" y="23" width="4" height="13" rx="2" fill=move || seg(2) />
                                                        <rect x="6" y="36" width="14" height="4" rx="2" fill=move || seg(3) />
                                                        <rect x="3" y="23" width="4" height="13" rx="2" fill=move || seg(4) />
                                                        <rect x="3" y="5" width="4" height="13" rx="2" fill=move || seg(5) />
                                                        <rect x="6" y="19" width="14" height="4" rx="2" fill=move || seg(6) />
                                                    }
                                                }
                                            </svg>
                                        }
                                            .into_any(),
                                        PartKind::Display => view! {
                                            <span class="grid min-h-[34px] min-w-[110px] place-items-center rounded-[4px] bg-[#0d1a12] px-2 py-1 font-mono text-caption text-[#3ddc84] ring-1 ring-[#1d4a2f]">
                                                {move || {
                                                    let text = state.sim_display.get();
                                                    if text.is_empty() {
                                                        "········".to_string()
                                                    } else {
                                                        text
                                                    }
                                                }}
                                            </span>
                                        }
                                            .into_any(),
                                        PartKind::Pot => view! {
                                            <input
                                                type="range"
                                                min="0"
                                                max="255"
                                                value="128"
                                                on:pointerdown=move |event: ev::PointerEvent| {
                                                    event.stop_propagation();
                                                }
                                                on:input=move |event: ev::Event| {
                                                    if let Ok(value) =
                                                        event_target_value(&event).parse::<u8>()
                                                    {
                                                        controller::sim_pot(
                                                            state, pins[0], value,
                                                        );
                                                    }
                                                }
                                                class="w-[80px] accent-[#c9a227]"
                                            />
                                        }
                                            .into_any(),
                                        PartKind::Button => view! {
                                            <span
                                                on:pointerdown=move |event: ev::PointerEvent| {
                                                    if running.get_untracked() {
                                                        event.stop_propagation();
                                                        controller::sim_press(
                                                            state, pins[0], true,
                                                        );
                                                    }
                                                }
                                                on:pointerup=move |_| {
                                                    if running.get_untracked() {
                                                        controller::sim_press(
                                                            state, pins[0], false,
                                                        );
                                                    }
                                                }
                                                class=move || {
                                                    let pressed = running.get()
                                                        && level(pins[0]);
                                                    format!(
                                                        "grid size-5 shrink-0 cursor-pointer place-items-center rounded-[5px] ring-1 ring-[#5a626e] {}",
                                                        if pressed {
                                                            "bg-rust"
                                                        } else {
                                                            "bg-[#3a404a]"
                                                        },
                                                    )
                                                }
                                            >
                                                <span class="size-2 rounded-full bg-[#9aa3b0]" />
                                            </span>
                                        }
                                            .into_any(),
                                    };

                                    view! {
                                        <div
                                            on:pointerdown=move |event: ev::PointerEvent| {
                                                event.prevent_default();
                                                event.stop_propagation();
                                                selected.set(Some(index));
                                                selected_wire.set(None);
                                                if matches!(grab_kind, PartKind::Button)
                                                    && running.get_untracked()
                                                {
                                                    return;
                                                }
                                                checkpoint();
                                                let world = to_world(
                                                    f64::from(event.client_x()),
                                                    f64::from(event.client_y()),
                                                );
                                                drag.set(Some(Drag::Part {
                                                    index,
                                                    dx: world.0 - x,
                                                    dy: world.1 - y,
                                                }));
                                            }
                                            class=move || {
                                                let ring = if is_selected.get() {
                                                    "ring-2 ring-rust"
                                                } else {
                                                    "ring-1 ring-[#515a68]"
                                                };
                                                format!(
                                                    "absolute flex cursor-grab items-center gap-1.5 rounded-[8px] bg-[#2c313a] px-1.5 py-1 select-none {ring}",
                                                )
                                            }
                                            style=format!(
                                                "left: {x}px; top: {y}px; width: {width}px",
                                            )
                                        >
                                            {face}
                                            <span class="min-w-0 flex-1 truncate font-mono text-caption text-[#d7dce3]">
                                                {label}
                                            </span>
                                            // pin stubs: the gold dots wires
                                            // pull out of
                                            {(!matches!(kind, PartKind::Display))
                                                .then(|| {
                                                    (0..wires)
                                                        .map(|slot| {
                                                            let unwired =
                                                                pins[slot] == UNWIRED;
                                                            let top = 8.0 + slot as f64 * 6.0;
                                                            view! {
                                                                <span
                                                                    title=if unwired {
                                                                        "Drag to a chip pin to wire this"
                                                                    } else {
                                                                        "Drag to rewire"
                                                                    }
                                                                    on:pointerdown=move |event: ev::PointerEvent| {
                                                                        event.prevent_default();
                                                                        event.stop_propagation();
                                                                        selected.set(Some(index));
                                                                        selected_wire.set(None);
                                                                        drag.set(Some(Drag::Wire {
                                                                            part: index,
                                                                            slot,
                                                                        }));
                                                                    }
                                                                    class=format!(
                                                                        "absolute size-[9px] cursor-crosshair rounded-full ring-1 ring-[#101216] {}",
                                                                        if unwired {
                                                                            "bg-[#e0a838] animate-pulse"
                                                                        } else {
                                                                            "bg-[#c9a227]"
                                                                        },
                                                                    )
                                                                    style=format!(
                                                                        "right: -5px; top: {top}px",
                                                                    )
                                                                />
                                                            }
                                                        })
                                                        .collect_view()
                                                })}
                                        </div>
                                    }
                                })
                                .collect_view()
                        }}
                    </div>
                </div>

                <div class="flex w-[190px] flex-none flex-col border-l border-line bg-sidebar">
                    {move || {
                        // A selected wire outranks a selected part.
                        if let Some((part_index, slot)) = selected_wire.get() {
                            let part = parts.with(|l| l.get(part_index).cloned());
                            let Some(part) = part else {
                                return ().into_any();
                            };
                            let pin = part.pins[slot];
                            let bends = part.waypoints[slot].len();
                            return view! {
                                <div class="flex flex-col gap-2 p-3">
                                    <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                        "Wire"
                                    </span>
                                    <p class="text-footnote text-label-2">
                                        {format!("{} → GPIO{pin}", part.label)}
                                    </p>
                                    <p class="text-footnote text-label-4">
                                        {format!(
                                            "{bends} bend(s) — double-click the wire to add one, drag the squares to move them",
                                        )}
                                    </p>
                                    <button
                                        type="button"
                                        on:click=move |_| {
                                            checkpoint();
                                            parts.update(|list| {
                                                if let Some(p) = list.get_mut(part_index) {
                                                    p.waypoints[slot].clear();
                                                }
                                            });
                                            dirty.set(true);
                                        }
                                        class="rounded-[6px] px-2 py-1 text-footnote text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label"
                                    >
                                        "Straighten"
                                    </button>
                                    <button
                                        type="button"
                                        on:click=move |_| delete_selection()
                                        class="rounded-[6px] px-2 py-1 text-footnote text-crimson ring-1 ring-line hover:bg-sunken"
                                    >
                                        "Disconnect (Del)"
                                    </button>
                                </div>
                            }
                                .into_any();
                        }
                        let Some(index) = selected.get() else {
                            return view! {
                                <p class="p-3 text-footnote text-label-4">
                                    "Select a part or a wire. Drag a part's gold stub to a \
                                     chip pin to wire it — wiring is what sets the pin. \
                                     Wheel zooms, background drags, Del removes."
                                </p>
                            }
                                .into_any();
                        };
                        let Some(part) = parts.with(|list| list.get(index).cloned()) else {
                            return ().into_any();
                        };

                        let pin_text = move |slot: usize| {
                            let pin = parts
                                .with_untracked(|list| {
                                    list.get(index).map(|p| p.pins[slot]).unwrap_or(UNWIRED)
                                });
                            if pin == UNWIRED {
                                "—".to_string()
                            } else {
                                pin.to_string()
                            }
                        };

                        view! {
                            <div class="flex flex-col gap-2 p-3">
                                <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                    {match part.kind {
                                        PartKind::Led { .. } => "LED",
                                        PartKind::Button => "Button",
                                        PartKind::Rgb => "RGB LED",
                                        PartKind::Seven => "7-segment",
                                        PartKind::Display => "Display",
                                        PartKind::Pot => "Potentiometer",
                                    }}
                                </span>
                                {(!matches!(part.kind, PartKind::Display))
                                    .then(|| {
                                        let names: &[&str] = match part.kind {
                                            PartKind::Rgb => &["r", "g", "b"],
                                            PartKind::Seven => {
                                                &["a", "b", "c", "d", "e", "f", "g"]
                                            }
                                            _ => &["pin"],
                                        };
                                        view! {
                                            <div class="flex flex-col gap-1">
                                                {names
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(slot, name)| {
                                                        view! {
                                                            <p class="flex items-center gap-2 font-mono text-footnote text-label-2">
                                                                <span class="w-[3ch] text-label-3">
                                                                    {*name}
                                                                </span>
                                                                <span>{pin_text(slot)}</span>
                                                            </p>
                                                        }
                                                    })
                                                    .collect_view()}
                                                <p class="text-caption leading-snug text-label-4">
                                                    "wire pins by dragging the gold stubs to \
                                                     the chip"
                                                </p>
                                            </div>
                                        }
                                    })}
                                {matches!(part.kind, PartKind::Display)
                                    .then(|| {
                                        view! {
                                            <p class="text-footnote leading-snug text-label-4">
                                                "Shows whatever the firmware prints as \
                                                 [rusty:disp] <text> — no pins to wire."
                                            </p>
                                        }
                                    })}
                                {matches!(part.kind, PartKind::Led { .. })
                                    .then(|| {
                                        view! {
                                            <div class="flex items-center gap-1.5">
                                                {[
                                                    ("green", "bg-[#3ddc84]"),
                                                    ("blue", "bg-[#4aa8ff]"),
                                                    ("red", "bg-[#ff5c5c]"),
                                                    ("yellow", "bg-[#ffd75c]"),
                                                ]
                                                    .into_iter()
                                                    .map(|(name, swatch)| {
                                                        view! {
                                                            <button
                                                                type="button"
                                                                title=name
                                                                on:click=move |_| {
                                                                    checkpoint();
                                                                    parts.update(|list| {
                                                                        if let Some(part) =
                                                                            list.get_mut(index)
                                                                        {
                                                                            part.kind = PartKind::Led {
                                                                                color: name.to_string(),
                                                                            };
                                                                        }
                                                                    });
                                                                    dirty.set(true);
                                                                }
                                                                class=format!(
                                                                    "size-5 rounded-full ring-1 ring-line hover:ring-2 {swatch}",
                                                                )
                                                            />
                                                        }
                                                    })
                                                    .collect_view()}
                                            </div>
                                        }
                                    })}
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        checkpoint();
                                        parts.update(|list| {
                                            if index < list.len() {
                                                list.remove(index);
                                            }
                                        });
                                        selected.set(None);
                                        dirty.set(true);
                                    }
                                    class="rounded-[6px] px-2 py-1 text-footnote text-crimson ring-1 ring-line hover:bg-sunken"
                                >
                                    "Remove (Del)"
                                </button>
                            </div>
                        }
                            .into_any()
                    }}
                </div>
            </div>
        </div>
    }
}
