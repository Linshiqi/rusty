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
}

fn pins3(a: u8, b: u8, c: u8) -> [u8; 7] {
    [a, b, c, 0, 0, 0, 0]
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
        });
    }
    for button in &board.buttons {
        out.push(EditPart {
            kind: PartKind::Button,
            pins: pins3(button.pin, 0, 0),
            label: button.label.clone(),
            x: button.x.unwrap_or(60.0),
            y: button.y.unwrap_or(180.0),
        });
    }
    for rgb in &board.rgbs {
        out.push(EditPart {
            kind: PartKind::Rgb,
            pins: pins3(rgb.r, rgb.g, rgb.b),
            label: rgb.label.clone(),
            x: rgb.x.unwrap_or(60.0),
            y: rgb.y.unwrap_or(240.0),
        });
    }
    for seven in &board.sevens {
        out.push(EditPart {
            kind: PartKind::Seven,
            pins: seven.pins,
            label: seven.label.clone(),
            x: seven.x.unwrap_or(160.0),
            y: seven.y.unwrap_or(60.0),
        });
    }
    for display in &board.displays {
        out.push(EditPart {
            kind: PartKind::Display,
            pins: [0; 7],
            label: display.label.clone(),
            x: display.x.unwrap_or(160.0),
            y: display.y.unwrap_or(160.0),
        });
    }
    for pot in &board.pots {
        out.push(EditPart {
            kind: PartKind::Pot,
            pins: pins3(pot.pin, 0, 0),
            label: pot.label.clone(),
            x: pot.x.unwrap_or(60.0),
            y: pot.y.unwrap_or(280.0),
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
            }),
            PartKind::Button => board.buttons.push(rusty_embed::SimButton {
                pin: part.pins[0],
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
            }),
            PartKind::Rgb => board.rgbs.push(rusty_embed::SimRgb {
                r: part.pins[0],
                g: part.pins[1],
                b: part.pins[2],
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
            }),
            PartKind::Seven => board.sevens.push(rusty_embed::SimSeven {
                pins: part.pins,
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
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
}

const SNAP: f64 = 8.0;
const KIT_W: f64 = 150.0;
const KIT_H: f64 = 230.0;
/// Pin rows per side of the schematic devkit.
const KIT_ROWS: usize = 15;

fn snap(value: f64) -> f64 {
    (value / SNAP).round() * SNAP
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
    let drag = RwSignal::new(None::<Drag>);
    // View transform: world → screen is (x * k + tx, y * k + ty).
    let view = RwSignal::new((0.0f64, 0.0f64, 1.0f64));
    let canvas: NodeRef<leptos::html::Div> = NodeRef::new();

    // ── undo/redo: snapshots of (parts, kit) ────────────────────────────
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

    let add_part = move |kind: PartKind, label_stub: String| {
        checkpoint();
        parts.update(|list| {
            let pin = 2 + list.len() as u8;
            let label = match &kind {
                PartKind::Led { .. } => format!("GPIO{pin}"),
                PartKind::Button => format!("BTN{pin}"),
                PartKind::Rgb => label_stub.clone(),
                PartKind::Seven => "7SEG".to_string(),
                PartKind::Display => "DISPLAY".to_string(),
                PartKind::Pot => format!("POT{pin}"),
            };
            list.push(EditPart {
                kind,
                pins: [
                    pin,
                    pin.saturating_add(1),
                    pin.saturating_add(2),
                    pin.saturating_add(3),
                    pin.saturating_add(4),
                    pin.saturating_add(5),
                    pin.saturating_add(6),
                ],
                label,
                x: snap(60.0 + (list.len() as f64 * 16.0) % 90.0),
                y: snap(40.0 + (list.len() as f64 * 52.0) % 240.0),
            });
            selected.set(Some(list.len() - 1));
        });
        dirty.set(true);
    };

    let save = move |_| {
        let board = board_of(&chip, kit_pos.get_untracked(), &parts.get_untracked());
        controller::save_sim_board(state, board, dirty);
    };

    // screen → world through the current view transform.
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
                    "pin levels as reported by the firmware over serial"
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
                                    title=format!("Add a {color} LED")
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

                // ── the sheet ────────────────────────────────────────────
                <div
                    node_ref=canvas
                    tabindex="0"
                    on:keydown=move |event: ev::KeyboardEvent| {
                        if event.ctrl_key() && event.key().eq_ignore_ascii_case("z") {
                            event.prevent_default();
                            if event.shift_key() { redo() } else { undo() }
                        } else if event.ctrl_key() && event.key().eq_ignore_ascii_case("y") {
                            event.prevent_default();
                            redo();
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
                            // Anchor the zoom at the cursor: the world point
                            // under it stays under it.
                            *tx = cx - (cx - *tx) * real;
                            *ty = cy - (cy - *ty) * real;
                            *k = next;
                        });
                    }
                    on:pointerdown=move |event: ev::PointerEvent| {
                        // Background press pans the sheet.
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
                        }
                    }
                    on:pointermove=move |event: ev::PointerEvent| {
                        let Some(current) = drag.get_untracked() else {
                            return;
                        };
                        match current {
                            Drag::Pan { start_tx, start_ty, px, py } => {
                                view.update(|(tx, ty, _)| {
                                    *tx = start_tx + f64::from(event.client_x()) - px;
                                    *ty = start_ty + f64::from(event.client_y()) - py;
                                });
                            }
                            Drag::Part { index, dx, dy } => {
                                let (wx, wy) = to_world(
                                    f64::from(event.client_x()),
                                    f64::from(event.client_y()),
                                );
                                parts.update(|list| {
                                    if let Some(part) = list.get_mut(index) {
                                        part.x = snap(wx - dx);
                                        part.y = snap(wy - dy);
                                    }
                                });
                                dirty.set(true);
                            }
                            Drag::Kit { dx, dy } => {
                                let (wx, wy) = to_world(
                                    f64::from(event.client_x()),
                                    f64::from(event.client_y()),
                                );
                                kit_pos.set((snap(wx - dx), snap(wy - dy)));
                                dirty.set(true);
                            }
                        }
                    }
                    on:pointerup=move |_| drag.set(None)
                    on:pointerleave=move |_| drag.set(None)
                    class="relative min-w-0 flex-1 overflow-hidden bg-[#101216] outline-none"
                >
                    // Everything on the sheet lives in world coordinates in
                    // this one transformed layer — parts, kit, wires, grid.
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
                                {move || {
                                    let (kx, ky) = kit_pos.get();
                                    let mut slot = 0usize;
                                    parts
                                        .get()
                                        .iter()
                                        .flat_map(|part| {
                                            let wires = part.kind.wires();
                                            let sx = part.x + part.kind.width();
                                            let sy = part.y + 14.0;
                                            let out: Vec<_> = (0..wires)
                                                .map(|w| {
                                                    let this = slot + w;
                                                    // Left column first, right
                                                    // column after it fills.
                                                    let (px, py, from_right) =
                                                        if this < KIT_ROWS {
                                                            (
                                                                kx + 10.0,
                                                                ky + 16.0 + this as f64 * 14.0,
                                                                false,
                                                            )
                                                        } else {
                                                            (
                                                                kx + KIT_W - 10.0,
                                                                ky + 16.0
                                                                    + (this - KIT_ROWS) as f64
                                                                        * 14.0,
                                                                true,
                                                            )
                                                        };
                                                    let wy = sy + w as f64 * 6.0;
                                                    // Stagger the vertical runs
                                                    // so parallel wires stay
                                                    // parallel, not stacked.
                                                    let mid = if from_right {
                                                        px + 24.0 + this as f64 * 8.0
                                                    } else {
                                                        px - 24.0 - this as f64 * 8.0
                                                    };
                                                    let pin = part.pins[w];
                                                    view! {
                                                        <polyline
                                                            points=format!(
                                                                "{sx},{wy} {mid},{wy} {mid},{py} {px},{py}",
                                                            )
                                                            fill="none"
                                                            stroke="#7d8694"
                                                            stroke-width="1.6"
                                                        />
                                                        <circle cx=sx cy=wy r="2.6" fill="#c9a227" />
                                                        <circle cx=px cy=py r="2.6" fill="#c9a227" />
                                                        <text
                                                            x=if from_right { px + 8.0 } else { px - 8.0 }
                                                            y=py + 3.0
                                                            text-anchor=if from_right { "start" } else { "end" }
                                                            font-family="ui-monospace"
                                                            font-size="9"
                                                            fill="#98a1ae"
                                                        >
                                                            {pin.to_string()}
                                                        </text>
                                                    }
                                                })
                                                .collect();
                                            slot += wires;
                                            out
                                        })
                                        .collect_view()
                                }}
                            </g>
                        </svg>

                        // the devkit — a part like any other, so it drags
                        {move || {
                            let (kx, ky) = kit_pos.get();
                            view! {
                                <div
                                    on:pointerdown=move |event: ev::PointerEvent| {
                                        event.prevent_default();
                                        event.stop_propagation();
                                        checkpoint();
                                        let (wx, wy) = to_world(
                                            f64::from(event.client_x()),
                                            f64::from(event.client_y()),
                                        );
                                        let (kx, ky) = kit_pos.get_untracked();
                                        drag.set(Some(Drag::Kit {
                                            dx: wx - kx,
                                            dy: wy - ky,
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
                                        {(0..KIT_ROWS)
                                            .map(|i| {
                                                let y = 16 + i as i32 * 14;
                                                view! {
                                                    <circle cx="10" cy=y r="3.2" fill="#c9a227" />
                                                    <circle cx=KIT_W - 10.0 cy=y r="3.2" fill="#c9a227" />
                                                }
                                            })
                                            .collect_view()}
                                        <rect x="34" y="12" width=KIT_W - 68.0 height="84" rx="4" fill="#2e333b" stroke="#4a515d" />
                                        <text x=KIT_W / 2.0 y="58" text-anchor="middle" font-family="ui-monospace" font-size="14" fill="#aab3c0">
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
                                                if matches!(grab_kind, PartKind::Button)
                                                    && running.get_untracked()
                                                {
                                                    return;
                                                }
                                                checkpoint();
                                                let (wx, wy) = to_world(
                                                    f64::from(event.client_x()),
                                                    f64::from(event.client_y()),
                                                );
                                                drag.set(Some(Drag::Part {
                                                    index,
                                                    dx: wx - x,
                                                    dy: wy - y,
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
                                        </div>
                                    }
                                })
                                .collect_view()
                        }}
                    </div>
                </div>

                <div class="flex w-[190px] flex-none flex-col border-l border-line bg-sidebar">
                    {move || {
                        let Some(index) = selected.get() else {
                            return view! {
                                <p class="p-3 text-footnote text-label-4">
                                    "Select a part to edit its pins and colour. Drag the \
                                     background to pan, wheel to zoom, drag the chip to \
                                     move it. While the simulation runs, buttons press \
                                     instead of dragging."
                                </p>
                            }
                                .into_any();
                        };
                        let Some(part) = parts.with(|list| list.get(index).cloned()) else {
                            return ().into_any();
                        };

                        let pin_field = move |slot: usize, name: &'static str| {
                            let value = parts
                                .with_untracked(|list| {
                                    list.get(index).map(|p| p.pins[slot]).unwrap_or(0)
                                });
                            view! {
                                <label class="flex items-center gap-2 text-footnote text-label-2">
                                    {name}
                                    <input
                                        type="number"
                                        min="0"
                                        max="48"
                                        prop:value=value.to_string()
                                        on:change=move |event: ev::Event| {
                                            let text = event_target_value(&event);
                                            if let Ok(pin) = text.trim().parse::<u8>() {
                                                checkpoint();
                                                parts.update(|list| {
                                                    if let Some(part) = list.get_mut(index) {
                                                        part.pins[slot] = pin;
                                                        if slot == 0
                                                            && matches!(
                                                                part.kind,
                                                                PartKind::Led { .. }
                                                            )
                                                        {
                                                            part.label = format!("GPIO{pin}");
                                                        }
                                                        if slot == 0
                                                            && matches!(part.kind, PartKind::Button)
                                                        {
                                                            part.label = format!("BTN{pin}");
                                                        }
                                                        if slot == 0
                                                            && matches!(part.kind, PartKind::Pot)
                                                        {
                                                            part.label = format!("POT{pin}");
                                                        }
                                                    }
                                                });
                                                dirty.set(true);
                                            }
                                        }
                                        class="w-[7ch] rounded-[5px] bg-sunken px-1.5 py-0.5 font-mono text-footnote text-label"
                                    />
                                </label>
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
                                {match part.kind.clone() {
                                    PartKind::Rgb => view! {
                                        {pin_field(0, "r")}
                                        {pin_field(1, "g")}
                                        {pin_field(2, "b")}
                                    }
                                        .into_any(),
                                    PartKind::Seven => view! {
                                        {pin_field(0, "a")}
                                        {pin_field(1, "b")}
                                        {pin_field(2, "c")}
                                        {pin_field(3, "d")}
                                        {pin_field(4, "e")}
                                        {pin_field(5, "f")}
                                        {pin_field(6, "g")}
                                    }
                                        .into_any(),
                                    PartKind::Display => view! {
                                        <p class="text-footnote leading-snug text-label-4">
                                            "Shows whatever the firmware prints as \
                                             [rusty:disp] <text> — no pins to wire."
                                        </p>
                                    }
                                        .into_any(),
                                    _ => pin_field(0, "pin").into_any(),
                                }}
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
                                    "Remove"
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
