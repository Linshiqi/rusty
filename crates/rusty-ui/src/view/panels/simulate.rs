//! Running firmware with no hardware on the desk — and wiring the desk up.
//!
//! The page is a small board editor in the Wokwi shape: a component library
//! on the left, a canvas with the devkit on the right, a toolbar above.
//! LEDs are added from the library, dragged into place, given a pin and a
//! colour, and saved into the project's `.rusty/sim.toml` — a file diffed
//! and reviewed like any other. At run time each LED lights from the pin
//! levels the firmware reports over serial, and the caption says exactly
//! that: the QEMU peripheral models expose no GPIO readback to do better.

use leptos::{ev, prelude::*};

use rusty_embed::{SimBoard, SimLed};

use crate::{controller, state::AppState, view::components::Empty};

/// Canvas geometry, shared by layout and the wire drawing.
const CANVAS_W: f64 = 560.0;
const CANVAS_H: f64 = 340.0;
const KIT_X: f64 = 360.0;
const KIT_Y: f64 = 30.0;

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
            <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-3">
                {(!missing.is_empty())
                    .then(|| {
                        view! {
                            <div class="flex max-w-[70ch] flex-col gap-2.5 rounded-[8px] bg-amber-fill px-4 py-3">
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
                    })
                    blocked=blocked
                />

                <details class="max-w-[76ch]">
                    <summary class="cursor-pointer text-footnote text-label-3 select-none hover:text-label-2">
                        "What Run does"
                    </summary>
                    <div class="mt-2 flex flex-col gap-2">
                        {plan
                            .steps
                            .iter()
                            .enumerate()
                            .map(|(index, step)| {
                                view! {
                                    <div class="rounded-[8px] bg-raised px-4 py-2.5 ring-1 ring-line">
                                        <div class="flex items-baseline gap-2">
                                            <span class="shrink-0 font-mono text-footnote text-label-3">
                                                {format!("{}.", index + 1)}
                                            </span>
                                            <code class="min-w-0 flex-1 font-mono text-footnote break-all select-text">
                                                {step.display.clone()}
                                            </code>
                                        </div>
                                        <p class="mt-1 pl-5 text-footnote leading-relaxed text-label-3">
                                            {step.rationale.clone()}
                                        </p>
                                    </div>
                                }
                            })
                            .collect_view()}
                    </div>
                </details>
            </div>
        }
        .into_any()
    }
}

/// The editor: library, canvas, toolbar. Local state until Save writes it
/// into `.rusty/sim.toml` and the plan reloads.
#[component]
fn BoardEditor(board: SimBoard, blocked: bool) -> impl IntoView {
    let state = AppState::expect();
    let running = state.session_running;
    let chip = board.chip.clone();
    let chip_label = board.chip.to_uppercase();

    // Positions materialise on load so dragging always has coordinates.
    let leds = RwSignal::new(
        board
            .leds
            .iter()
            .enumerate()
            .map(|(index, led)| SimLed {
                x: Some(led.x.unwrap_or(60.0)),
                y: Some(led.y.unwrap_or(40.0 + index as f64 * 56.0)),
                ..led.clone()
            })
            .collect::<Vec<_>>(),
    );
    let dirty = RwSignal::new(false);
    let selected = RwSignal::new(None::<usize>);
    // (index, grab offset x, grab offset y) while a drag is live.
    let dragging = RwSignal::new(None::<(usize, f64, f64)>);
    let canvas: NodeRef<leptos::html::Div> = NodeRef::new();

    let add_led = move |color: &'static str| {
        leds.update(|list| {
            let pin = 2 + list.len() as u8;
            list.push(SimLed {
                pin,
                color: color.to_string(),
                label: format!("GPIO{pin}"),
                x: Some(50.0 + (list.len() as f64 * 14.0) % 80.0),
                y: Some(40.0 + (list.len() as f64 * 48.0) % 220.0),
            });
            selected.set(Some(list.len() - 1));
        });
        dirty.set(true);
    };

    let save = move |_| {
        let board = SimBoard {
            chip: chip.clone(),
            leds: leds.get_untracked(),
        };
        controller::save_sim_board(state, board, dirty);
    };

    let library = [
        ("green", "bg-[#3ddc84]"),
        ("blue", "bg-[#4aa8ff]"),
        ("red", "bg-[#ff5c5c]"),
        ("yellow", "bg-[#ffd75c]"),
    ];

    view! {
        <div class="flex w-fit flex-col gap-2">
            <div class="flex items-center gap-2">
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
                                    "running — serial output is in the panel below"
                                </span>
                            }
                        })
                }}
            </div>

            <div class="flex gap-2">
                <div class="flex w-[120px] flex-none flex-col gap-1 rounded-[10px] bg-sidebar p-2 ring-1 ring-line">
                    <span class="px-1 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        "Parts"
                    </span>
                    {library
                        .into_iter()
                        .map(|(color, swatch)| {
                            view! {
                                <button
                                    type="button"
                                    title=format!("Add a {color} LED")
                                    on:click=move |_| add_led(color)
                                    class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                                >
                                    <span class=format!("size-3.5 rounded-full {swatch}") />
                                    <span>{format!("{color} LED")}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                    <p class="mt-1 px-1 text-caption leading-snug text-label-4">
                        "buttons and displays: coming"
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
                            .clamp(4.0, CANVAS_W - 60.0);
                        let y = (f64::from(event.client_y()) - rect.top() - grab_y)
                            .clamp(4.0, CANVAS_H - 30.0);
                        leds.update(|list| {
                            if let Some(led) = list.get_mut(index) {
                                led.x = Some(x);
                                led.y = Some(y);
                            }
                        });
                        dirty.set(true);
                    }
                    on:pointerup=move |_| dragging.set(None)
                    on:pointerleave=move |_| dragging.set(None)
                    class="relative flex-none overflow-hidden rounded-[12px] bg-sunken ring-1 ring-line"
                    style=format!("width: {CANVAS_W}px; height: {CANVAS_H}px")
                >
                    <svg
                        class="pointer-events-none absolute inset-0"
                        width=CANVAS_W
                        height=CANVAS_H
                    >
                        {move || {
                            leds.get()
                                .iter()
                                .enumerate()
                                .map(|(index, led)| {
                                    let x = led.x.unwrap_or(60.0) + 10.0;
                                    let y = led.y.unwrap_or(40.0) + 10.0;
                                    let pin_y = KIT_Y + 18.0 + (index as f64 % 14.0) * 14.0;
                                    let pin_x = KIT_X + 10.0;
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

                    <svg
                        class="pointer-events-none absolute"
                        style=format!("left: {KIT_X}px; top: {KIT_Y}px")
                        width="150"
                        height="230"
                        viewBox="0 0 150 230"
                    >
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

                    {move || {
                        leds.get()
                            .iter()
                            .enumerate()
                            .map(|(index, led)| {
                                let pin = led.pin;
                                let color = led.color.clone();
                                let label = led.label.clone();
                                let x = led.x.unwrap_or(60.0);
                                let y = led.y.unwrap_or(40.0);
                                let lit = Signal::derive(move || {
                                    state
                                        .sim_gpio
                                        .with(|gpio| gpio.get(&pin).copied().unwrap_or(false))
                                });
                                let is_selected =
                                    Signal::derive(move || selected.get() == Some(index));
                                view! {
                                    <div
                                        on:pointerdown=move |event: ev::PointerEvent| {
                                            event.prevent_default();
                                            selected.set(Some(index));
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
                                        <span class=move || {
                                            let base = "size-4 rounded-full transition-all duration-150";
                                            let hue = match (color.as_str(), lit.get()) {
                                                ("green", true) => "bg-[#3ddc84] shadow-[0_0_12px_3px_rgba(61,220,132,0.55)]",
                                                ("green", false) => "bg-[#1d4a2f]",
                                                ("blue", true) => "bg-[#4aa8ff] shadow-[0_0_12px_3px_rgba(74,168,255,0.55)]",
                                                ("blue", false) => "bg-[#1d3350]",
                                                ("red", true) => "bg-[#ff5c5c] shadow-[0_0_12px_3px_rgba(255,92,92,0.55)]",
                                                ("red", false) => "bg-[#4a1d1d]",
                                                ("yellow", true) => "bg-[#ffd75c] shadow-[0_0_12px_3px_rgba(255,215,92,0.5)]",
                                                ("yellow", false) => "bg-[#4a3f1d]",
                                                (_, true) => "bg-label",
                                                (_, false) => "bg-line-strong",
                                            };
                                            format!("{base} {hue}")
                                        } />
                                        <span class="font-mono text-caption text-label-3">
                                            {label}
                                        </span>
                                    </div>
                                }
                            })
                            .collect_view()
                    }}
                </div>

                {move || {
                    let index = selected.get()?;
                    let led = leds.with(|list| list.get(index).cloned())?;
                    Some(
                        view! {
                            <div class="flex w-[150px] flex-none flex-col gap-2 rounded-[10px] bg-sidebar p-2.5 ring-1 ring-line">
                                <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                    "LED"
                                </span>
                                <label class="flex items-center gap-2 text-footnote text-label-2">
                                    "pin"
                                    <input
                                        type="number"
                                        min="0"
                                        max="48"
                                        prop:value=led.pin.to_string()
                                        on:change=move |event: ev::Event| {
                                            let value = event_target_value(&event);
                                            if let Ok(pin) = value.trim().parse::<u8>() {
                                                leds.update(|list| {
                                                    if let Some(led) = list.get_mut(index) {
                                                        led.pin = pin;
                                                        led.label = format!("GPIO{pin}");
                                                    }
                                                });
                                                dirty.set(true);
                                            }
                                        }
                                        class="w-[7ch] rounded-[5px] bg-sunken px-1.5 py-0.5 font-mono text-footnote text-label"
                                    />
                                </label>
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
                                                        leds.update(|list| {
                                                            if let Some(led) = list.get_mut(index) {
                                                                led.color = name.to_string();
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
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        leds.update(|list| {
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
                        },
                    )
                }}
            </div>
            <p class="text-caption text-label-4">
                "pin levels as reported by the firmware over serial"
            </p>
        </div>
    }
}
