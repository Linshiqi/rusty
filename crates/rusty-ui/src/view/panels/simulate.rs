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
                        leds: Vec::new(),
                        buttons: Vec::new(),
                        rgbs: Vec::new(),
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
}

#[derive(Clone, PartialEq)]
struct EditPart {
    kind: PartKind,
    /// pins[0] is the pin for LEDs and buttons; RGB uses all three.
    pins: [u8; 3],
    label: String,
    x: f64,
    y: f64,
}

fn parts_of(board: &SimBoard) -> Vec<EditPart> {
    let mut out = Vec::new();
    for (index, led) in board.leds.iter().enumerate() {
        out.push(EditPart {
            kind: PartKind::Led {
                color: led.color.clone(),
            },
            pins: [led.pin, 0, 0],
            label: led.label.clone(),
            x: led.x.unwrap_or(60.0),
            y: led.y.unwrap_or(40.0 + index as f64 * 56.0),
        });
    }
    for button in &board.buttons {
        out.push(EditPart {
            kind: PartKind::Button,
            pins: [button.pin, 0, 0],
            label: button.label.clone(),
            x: button.x.unwrap_or(60.0),
            y: button.y.unwrap_or(180.0),
        });
    }
    for rgb in &board.rgbs {
        out.push(EditPart {
            kind: PartKind::Rgb,
            pins: [rgb.r, rgb.g, rgb.b],
            label: rgb.label.clone(),
            x: rgb.x.unwrap_or(60.0),
            y: rgb.y.unwrap_or(240.0),
        });
    }
    out
}

fn board_of(chip: &str, parts: &[EditPart]) -> SimBoard {
    let mut board = SimBoard {
        chip: chip.to_string(),
        leds: Vec::new(),
        buttons: Vec::new(),
        rgbs: Vec::new(),
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
    let dirty = RwSignal::new(false);
    let selected = RwSignal::new(None::<usize>);
    let dragging = RwSignal::new(None::<(usize, f64, f64)>);
    let canvas: NodeRef<leptos::html::Div> = NodeRef::new();
    let kit: NodeRef<leptos::html::Div> = NodeRef::new();

    let add_part = move |kind: PartKind, label_stub: String| {
        parts.update(|list| {
            let pin = 2 + list.len() as u8;
            let label = match &kind {
                PartKind::Led { .. } => format!("GPIO{pin}"),
                PartKind::Button => format!("BTN{pin}"),
                PartKind::Rgb => label_stub.clone(),
            };
            list.push(EditPart {
                kind,
                pins: [pin, pin + 1, pin + 2],
                label,
                x: 50.0 + (list.len() as f64 * 16.0) % 90.0,
                y: 40.0 + (list.len() as f64 * 52.0) % 240.0,
            });
            selected.set(Some(list.len() - 1));
        });
        dirty.set(true);
    };

    let save = move |_| {
        let board = board_of(&chip, &parts.get_untracked());
        controller::save_sim_board(state, board, dirty);
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex flex-none items-center gap-2 border-b border-line px-4 py-2">
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
                {move || {
                    running
                        .get()
                        .then(|| {
                            view! {
                                <span class="text-footnote text-label-3">
                                    "running — buttons are live, serial below"
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
                        title="Add a push button — pressing it sends B<pin>=1/0 into the firmware's UART"
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
                        title="Add an RGB LED — three pins, additive colour"
                        on:click=move |_| add_part(PartKind::Rgb, "RGB".to_string())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="size-3.5 rounded-full bg-[conic-gradient(#ff5c5c,#3ddc84,#4aa8ff,#ff5c5c)]" />
                        <span>"RGB LED"</span>
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
                                    title=format!(
                                        "{name} — defined in .rusty/parts/, lights from the gpio report channel",
                                    )
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
                    on:pointermove=move |event: ev::PointerEvent| {
                        let Some((index, grab_x, grab_y)) = dragging.get_untracked() else {
                            return;
                        };
                        let Some(element) = canvas.get_untracked() else {
                            return;
                        };
                        let rect = element.get_bounding_client_rect();
                        let x = (f64::from(event.client_x()) - rect.left() - grab_x)
                            .clamp(4.0, (rect.width() - 70.0).max(4.0));
                        let y = (f64::from(event.client_y()) - rect.top() - grab_y)
                            .clamp(4.0, (rect.height() - 30.0).max(4.0));
                        parts.update(|list| {
                            if let Some(part) = list.get_mut(index) {
                                part.x = x;
                                part.y = y;
                            }
                        });
                        dirty.set(true);
                    }
                    on:pointerup=move |_| dragging.set(None)
                    on:pointerleave=move |_| dragging.set(None)
                    class="relative min-w-0 flex-1 overflow-hidden bg-sunken"
                >
                    <svg class="pointer-events-none absolute inset-0 h-full w-full">
                        {move || {
                            let (kit_left, kit_top) = kit_origin(canvas, kit);
                            parts
                                .get()
                                .iter()
                                .enumerate()
                                .map(|(index, part)| {
                                    let x = part.x + 10.0;
                                    let y = part.y + 10.0;
                                    let pin_y = kit_top + 22.0 + (index as f64 % 14.0) * 14.0;
                                    let pin_x = kit_left + 20.0;
                                    let mid = (x + pin_x) / 2.0;
                                    view! {
                                        <polyline
                                            points=format!(
                                                "{x},{y} {mid},{y} {mid},{pin_y} {pin_x},{pin_y}",
                                            )
                                            fill="none"
                                            stroke="#4a4f58"
                                            stroke-width="2"
                                        />
                                    }
                                })
                                .collect_view()
                        }}
                    </svg>

                    <div node_ref=kit class="pointer-events-none absolute top-8 right-10">
                        <svg width="150" height="230" viewBox="0 0 150 230">
                            <rect x="10" y="6" width="130" height="218" rx="10" fill="#16181c" stroke="#2c2f36" />
                            {(0..14)
                                .map(|i| {
                                    let y = 22 + i * 14;
                                    view! {
                                        <circle cx="20" cy=y r="3.2" fill="#c9a227" />
                                        <circle cx="130" cy=y r="3.2" fill="#c9a227" />
                                    }
                                })
                                .collect_view()}
                            <rect x="38" y="14" width="74" height="86" rx="4" fill="#2a2d33" stroke="#3a3e46" />
                            <text x="75" y="66" text-anchor="middle" font-family="ui-monospace" font-size="14" fill="#8b909a">
                                {chip_label}
                            </text>
                            <rect x="60" y="206" width="30" height="16" rx="2" fill="#3a3e46" />
                        </svg>
                    </div>

                    {move || {
                        parts
                            .get()
                            .iter()
                            .enumerate()
                            .map(|(index, part)| {
                                let x = part.x;
                                let y = part.y;
                                let label = part.label.clone();
                                let kind = part.kind.clone();
                                let pins = part.pins;
                                let is_selected =
                                    Signal::derive(move || selected.get() == Some(index));
                                let level = move |pin: u8| {
                                    state
                                        .sim_gpio
                                        .with(|gpio| gpio.get(&pin).copied().unwrap_or(false))
                                };

                                let face = match kind.clone() {
                                    PartKind::Led { color } => view! {
                                        <span class=move || {
                                            format!(
                                                "size-4 rounded-full transition-all duration-150 {}",
                                                lamp_classes(&color, level(pins[0])),
                                            )
                                        } />
                                    }
                                        .into_any(),
                                    PartKind::Rgb => view! {
                                        <span
                                            class="size-4 rounded-full transition-all duration-150"
                                            style=move || rgb_style(
                                                level(pins[0]),
                                                level(pins[1]),
                                                level(pins[2]),
                                            )
                                        />
                                    }
                                        .into_any(),
                                    PartKind::Button => view! {
                                        <span
                                            on:pointerdown=move |event: ev::PointerEvent| {
                                                if running.get_untracked() {
                                                    event.stop_propagation();
                                                    controller::sim_press(state, pins[0], true);
                                                }
                                            }
                                            on:pointerup=move |_| {
                                                if running.get_untracked() {
                                                    controller::sim_press(state, pins[0], false);
                                                }
                                            }
                                            class=move || {
                                                let pressed = running.get()
                                                    && level(pins[0]);
                                                format!(
                                                    "grid size-5 cursor-pointer place-items-center rounded-[5px] ring-1 ring-line-strong {}",
                                                    if pressed { "bg-rust" } else { "bg-raised" },
                                                )
                                            }
                                        >
                                            <span class="size-2 rounded-full bg-label-3" />
                                        </span>
                                    }
                                        .into_any(),
                                };

                                view! {
                                    <div
                                        on:pointerdown=move |event: ev::PointerEvent| {
                                            event.prevent_default();
                                            selected.set(Some(index));
                                            // A live board's buttons press; a
                                            // powered-off board's parts drag.
                                            if matches!(kind, PartKind::Button)
                                                && running.get_untracked()
                                            {
                                                return;
                                            }
                                            let Some(element) = canvas.get_untracked() else {
                                                return;
                                            };
                                            let rect = element.get_bounding_client_rect();
                                            let grab_x =
                                                f64::from(event.client_x()) - rect.left() - x;
                                            let grab_y =
                                                f64::from(event.client_y()) - rect.top() - y;
                                            dragging.set(Some((index, grab_x, grab_y)));
                                        }
                                        class=move || {
                                            let ring = if is_selected.get() {
                                                "ring-2 ring-rust"
                                            } else {
                                                "ring-1 ring-line"
                                            };
                                            format!(
                                                "absolute flex cursor-grab items-center gap-1.5 rounded-[8px] bg-raised px-1.5 py-1 select-none {ring}",
                                            )
                                        }
                                        style=format!("left: {x}px; top: {y}px")
                                    >
                                        {face}
                                        <span class="font-mono text-caption text-label-3">
                                            {label}
                                        </span>
                                    </div>
                                }
                            })
                            .collect_view()
                    }}
                </div>

                <div class="flex w-[190px] flex-none flex-col border-l border-line bg-sidebar">
                    {move || {
                        let Some(index) = selected.get() else {
                            return view! {
                                <p class="p-3 text-footnote text-label-4">
                                    "Select a part to edit its pins and colour. While the \
                                     simulation runs, buttons press instead of dragging."
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
                                    }}
                                </span>
                                {match part.kind.clone() {
                                    PartKind::Rgb => view! {
                                        {pin_field(0, "r")}
                                        {pin_field(1, "g")}
                                        {pin_field(2, "b")}
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

/// The devkit's top-left, in canvas coordinates. Measured because the kit is
/// CSS-docked to the right edge and the canvas is whatever size the window
/// grants it. Recomputed whenever the wires redraw; a window resize catches
/// up on the next interaction, which is when anyone is looking.
fn kit_origin(
    canvas: NodeRef<leptos::html::Div>,
    kit: NodeRef<leptos::html::Div>,
) -> (f64, f64) {
    // Tracked reads, deliberately: the first wire paint happens before these
    // nodes mount, and an untracked read froze the fallback coordinates in
    // forever — wires that stopped in mid-air, attached to nothing.
    let (Some(canvas), Some(kit)) = (canvas.get(), kit.get()) else {
        return (360.0, 30.0);
    };
    let c = canvas.get_bounding_client_rect();
    let k = kit.get_bounding_client_rect();
    (k.left() - c.left(), k.top() - c.top())
}
