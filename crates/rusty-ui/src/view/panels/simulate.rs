//! Running firmware with no hardware on the desk.
//!
//! Espressif's QEMU boots the same merged flash image `espflash` would burn,
//! and its serial console streams into the dock — the Wokwi experience,
//! entirely local. The plan is shown before anything runs: three commands,
//! each with its why, plus honest refusals when the chip has no machine
//! model or a tool is missing.

use leptos::prelude::*;

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
            <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-4">
                {(!missing.is_empty())
                    .then(|| {
                        view! {
                            <div class="flex max-w-[70ch] flex-col gap-2 rounded-[8px] bg-amber-fill px-4 py-3">
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
                                                    {move || {
                                                        running
                                                            .get()
                                                            .then(|| {
                                                                view! {
                                                                    <span class="text-footnote text-label-3">
                                                                        "output in the panel below"
                                                                    </span>
                                                                }
                                                            })
                                                    }}
                                                </div>
                                                // Manual instructions earn
                                                // their place only after the
                                                // button has failed.
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

                <div class="flex max-w-[76ch] flex-col gap-2">
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

                {plan
                    .board
                    .clone()
                    .map(|board| {
                        view! { <BoardView board=board /> }
                    })}

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
                                <span class="text-footnote text-label-3">
                                    "running — serial output is in the panel below"
                                </span>
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
                </div>
            </div>
        }
        .into_any()
    }
}

/// A stylised devkit with the project's LEDs on it, lit from the pin levels
/// the firmware reports over serial. Not a photograph of a board — a truth
/// display: the caption says exactly whose word the light is.
#[component]
fn BoardView(board: rusty_embed::SimBoard) -> impl IntoView {
    let state = AppState::expect();
    let chip = board.chip.to_uppercase();

    view! {
        <div class="flex flex-col gap-1.5">
            <div class="flex items-center gap-6 rounded-[12px] bg-sunken px-6 py-5 ring-1 ring-line w-fit">
                // The LEDs, off-board like the classic wiring diagram.
                <div class="flex flex-col gap-4">
                    {board
                        .leds
                        .iter()
                        .map(|led| {
                            let pin = led.pin;
                            let color = led.color.clone();
                            let label = led.label.clone();
                            let lit = Signal::derive(move || {
                                state.sim_gpio.with(|gpio| gpio.get(&pin).copied().unwrap_or(false))
                            });
                            view! {
                                <div class="flex items-center gap-2.5">
                                    <div class=move || {
                                        let base = "size-5 rounded-full transition-all duration-150";
                                        let hue = match (color.as_str(), lit.get()) {
                                            ("green", true) => "bg-[#3ddc84] shadow-[0_0_14px_4px_rgba(61,220,132,0.55)]",
                                            ("green", false) => "bg-[#1d4a2f]",
                                            ("blue", true) => "bg-[#4aa8ff] shadow-[0_0_14px_4px_rgba(74,168,255,0.55)]",
                                            ("blue", false) => "bg-[#1d3350]",
                                            ("red", true) => "bg-[#ff5c5c] shadow-[0_0_14px_4px_rgba(255,92,92,0.55)]",
                                            ("red", false) => "bg-[#4a1d1d]",
                                            ("yellow", true) => "bg-[#ffd75c] shadow-[0_0_14px_4px_rgba(255,215,92,0.5)]",
                                            ("yellow", false) => "bg-[#4a3f1d]",
                                            (_, true) => "bg-label shadow-[0_0_14px_4px_rgba(255,255,255,0.4)]",
                                            (_, false) => "bg-line-strong",
                                        };
                                        format!("{base} {hue}")
                                    } />
                                    <span class="font-mono text-caption text-label-3">{label}</span>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>

                // The devkit, schematic rather than photographic.
                <svg width="150" height="230" viewBox="0 0 150 230" aria-hidden="true">
                    <rect x="10" y="6" width="130" height="218" rx="10" fill="#16181c" stroke="#2c2f36" />
                    // pin headers
                    {(0..14)
                        .map(|i| {
                            let y = 22 + i * 14;
                            view! {
                                <circle cx="20" cy=y r="3.2" fill="#c9a227" />
                                <circle cx="130" cy=y r="3.2" fill="#c9a227" />
                            }
                        })
                        .collect_view()}
                    // the module shield + antenna
                    <rect x="38" y="14" width="74" height="86" rx="4" fill="#2a2d33" stroke="#3a3e46" />
                    <path d="M44 22h62M44 30h62M44 38h62" stroke="#3a3e46" stroke-width="2" fill="none" />
                    <text x="75" y="66" text-anchor="middle" font-family="ui-monospace" font-size="14" fill="#8b909a">{chip}</text>
                    // usb notch
                    <rect x="60" y="206" width="30" height="16" rx="2" fill="#3a3e46" />
                </svg>
            </div>
            <p class="text-caption text-label-4">
                "pin levels as reported by the firmware over serial"
            </p>
        </div>
    }
}
