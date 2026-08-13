//! Pin waveforms from the running simulation.
//!
//! One lane per pin, step-drawn from the captured trace, on the time base
//! the trace itself declares — the firmware's systimer when the reports were
//! stamped, arrival time when they were not, and the header says which,
//! because a waveform that hides its clock is a waveform that lies.
//!
//! Wheel zooms about the cursor, dragging pans, and while the simulation
//! runs the view follows the newest edge until the first manual pan, the
//! way every logic-analyzer UI behaves. Export writes a VCD next to the
//! build artefacts, where PulseView and GTKWave can open it.

use leptos::{ev, prelude::*};

use crate::{
    controller,
    state::{AppState, TraceClock},
    view::components::{ContextMenu, MenuItem},
};

/// Lane geometry, in pixels.
const LANE_H: f64 = 30.0;
const LANE_GAP: f64 = 8.0;
const LABEL_W: f64 = 72.0;
const AXIS_H: f64 = 22.0;

#[component]
pub fn WavesTab() -> impl IntoView {
    let state = AppState::expect();

    // Visible window: (start µs, µs per pixel). Follow mode keeps the right
    // edge on the newest event until the user pans or zooms by hand.
    let window = RwSignal::new((0.0f64, 2000.0f64));
    let follow = RwSignal::new(true);
    let hover = RwSignal::new(None::<f64>);
    let pan = RwSignal::new(None::<(f64, f64)>);
    let plot: NodeRef<leptos::html::Div> = NodeRef::new();

    // The first paint happens before the plot mounts and would draw into a
    // fallback width; a nudge on mount re-runs it against the real box.
    Effect::new(move |_| {
        if plot.get().is_some() {
            window.update(|_| {});
        }
    });

    let plot_width = move || {
        plot.get()
            .map(|el| el.get_bounding_client_rect().width() - LABEL_W)
            .unwrap_or(600.0)
            .max(50.0)
    };

    // One framing routine for the button and the context menu — two copies
    // would drift the day one of them learns about margins.
    let fit = move || {
        let trace = state.sim_trace.get_untracked();
        let (Some(first), Some(last)) = (trace.events.first(), trace.events.last()) else {
            return;
        };
        let span = (last.0 - first.0).max(1) as f64;
        follow.set(false);
        window.set((first.0 as f64, span / plot_width()));
    };
    let menu = RwSignal::new(None::<(f64, f64)>);

    view! {
        <div
            class="flex min-h-0 flex-1 flex-col"
            on:contextmenu=move |event: ev::MouseEvent| {
                event.prevent_default();
                menu.set(Some((event.client_x() as f64, event.client_y() as f64)));
            }
        >
            <div class="flex flex-none items-center gap-2 border-b border-line px-3 py-1.5">
                {move || {
                    let trace = state.sim_trace.get();
                    let clock = match trace.clock {
                        Some(TraceClock::Firmware) => "clock: firmware systimer (µs)",
                        Some(TraceClock::Host) => "clock: host arrival time — firmware sent no stamps",
                        None => "no trace yet — run a simulation",
                    };
                    view! {
                        <span class="text-footnote text-label-3">{clock}</span>
                        {trace
                            .truncated
                            .then(|| {
                                view! {
                                    <span class="text-footnote text-amber">
                                        "oldest events dropped (cap)"
                                    </span>
                                }
                            })}
                        <span class="font-mono text-caption text-label-4">
                            {format!("{} edges", trace.events.len())}
                        </span>
                    }
                }}
                <span class="flex-1" />
                <button
                    type="button"
                    class=move || {
                        let base = "rounded-[5px] px-2 py-0.5 text-footnote ring-1 ring-line";
                        if follow.get() {
                            format!("{base} bg-selection text-rust")
                        } else {
                            format!("{base} text-label-3 hover:text-label")
                        }
                    }
                    on:click=move |_| follow.update(|f| *f = !*f)
                >
                    "Follow"
                </button>
                <button
                    type="button"
                    title="Frame the whole capture"
                    class="rounded-[5px] px-2 py-0.5 text-footnote text-label-3 ring-1 ring-line hover:text-label"
                    on:click=move |_| fit()
                >
                    "Fit"
                </button>
                <button
                    type="button"
                    title="Write target/rusty-sim/trace.vcd for PulseView"
                    class="rounded-[5px] px-2 py-0.5 text-footnote text-label-3 ring-1 ring-line hover:text-label"
                    disabled=move || state.sim_trace.with(|t| t.events.is_empty())
                    on:click=move |_| controller::export_vcd(state)
                >
                    "Export VCD"
                </button>
            </div>

            <div
                node_ref=plot
                class="relative min-h-0 flex-1 overflow-hidden select-none"
                on:wheel=move |event: ev::WheelEvent| {
                    event.prevent_default();
                    let Some(el) = plot.get_untracked() else { return };
                    let rect = el.get_bounding_client_rect();
                    let x = f64::from(event.client_x()) - rect.left() - LABEL_W;
                    if x < 0.0 {
                        return;
                    }
                    follow.set(false);
                    window.update(|(start, per_px)| {
                        let anchor = *start + x * *per_px;
                        let factor = if event.delta_y() < 0.0 { 1.0 / 1.3 } else { 1.3 };
                        *per_px = (*per_px * factor).clamp(0.05, 5_000_000.0);
                        *start = anchor - x * *per_px;
                    });
                }
                on:pointerdown=move |event: ev::PointerEvent| {
                    if event.button() != 0 {
                        return;
                    }
                    follow.set(false);
                    pan.set(Some((
                        f64::from(event.client_x()),
                        window.get_untracked().0,
                    )));
                }
                on:pointermove=move |event: ev::PointerEvent| {
                    let Some(el) = plot.get_untracked() else { return };
                    let rect = el.get_bounding_client_rect();
                    let x = f64::from(event.client_x()) - rect.left() - LABEL_W;
                    hover.set((x >= 0.0).then_some(x));
                    if let Some((grab_x, start0)) = pan.get_untracked() {
                        let (_, per_px) = window.get_untracked();
                        let dx = f64::from(event.client_x()) - grab_x;
                        window.update(|(start, _)| *start = start0 - dx * per_px);
                    }
                }
                on:pointerup=move |_| pan.set(None)
                on:pointerleave=move |_| {
                    pan.set(None);
                    hover.set(None);
                }
            >
                {move || {
                    let trace = state.sim_trace.get();
                    if trace.events.is_empty() {
                        return view! {
                            <p class="px-4 py-3 text-footnote text-label-3">
                                "Waveforms appear as the firmware reports pin changes — \
                                 run a simulation."
                            </p>
                        }
                            .into_any();
                    }

                    // Follow: keep the newest edge at the right margin.
                    let width = plot_width();
                    let (mut start, per_px) = window.get();
                    if follow.get()
                        && let Some((last, ..)) = trace.events.last()
                    {
                        start = *last as f64 - width * per_px * 0.85;
                        window.update_untracked(|w| w.0 = start);
                    }

                    // Lanes, in first-seen order — the order the firmware
                    // introduced them, which is the order the reader knows.
                    let mut pins: Vec<u8> = Vec::new();
                    for (_, pin, _) in &trace.events {
                        if !pins.contains(pin) {
                            pins.push(*pin);
                        }
                    }

                    let height = AXIS_H + pins.len() as f64 * (LANE_H + LANE_GAP);
                    let end = start + width * per_px;

                    // Axis ticks at a round step near 100px apart.
                    let raw = per_px * 100.0;
                    let step = 10f64.powf(raw.log10().floor());
                    let step = [step, step * 2.0, step * 5.0, step * 10.0]
                        .into_iter()
                        .find(|s| *s >= raw)
                        .unwrap_or(step);
                    let first_tick = (start / step).ceil() * step;
                    let ticks: Vec<f64> = (0..)
                        .map(|i| first_tick + i as f64 * step)
                        .take_while(|t| *t <= end)
                        .take(40)
                        .collect();

                    let fmt_us = move |us: f64| -> String {
                        if step >= 1_000_000.0 || us.abs() >= 10_000_000.0 {
                            format!("{:.2}s", us / 1_000_000.0)
                        } else if step >= 1_000.0 {
                            format!("{:.1}ms", us / 1_000.0)
                        } else {
                            format!("{us:.0}µs")
                        }
                    };

                    let lanes = pins
                        .iter()
                        .enumerate()
                        .map(|(lane, pin)| {
                            let top = AXIS_H + lane as f64 * (LANE_H + LANE_GAP);
                            let y_hi = top + 4.0;
                            let y_lo = top + LANE_H - 4.0;

                            // The level entering the window, then every edge
                            // inside it, as one step path.
                            let mut level = trace
                                .events
                                .iter()
                                .rev()
                                .find(|(t, p, _)| p == pin && (*t as f64) < start)
                                .map(|(_, _, l)| *l)
                                .unwrap_or(false);
                            let mut d = format!(
                                "M 0 {}",
                                if level { y_hi } else { y_lo },
                            );
                            for (t, p, l) in trace
                                .events
                                .iter()
                                .filter(|(t, p, _)| {
                                    p == pin
                                        && (*t as f64) >= start
                                        && (*t as f64) <= end
                                })
                            {
                                let x = ((*t as f64) - start) / per_px;
                                let _ = p;
                                if *l != level {
                                    d.push_str(&format!(
                                        " H {x:.1} V {}",
                                        if *l { y_hi } else { y_lo },
                                    ));
                                    level = *l;
                                }
                            }
                            d.push_str(&format!(" H {width:.1}"));

                            view! {
                                <text
                                    x="8"
                                    y=top + LANE_H / 2.0 + 4.0
                                    font-family="ui-monospace"
                                    font-size="11"
                                    fill="#98a1ae"
                                >
                                    {format!("GPIO{pin}")}
                                </text>
                                <g transform=format!("translate({LABEL_W}, 0)")>
                                    <line
                                        x1="0"
                                        y1=y_lo
                                        x2=width
                                        y2=y_lo
                                        stroke="#23262c"
                                        stroke-width="1"
                                    />
                                    <path
                                        d=d
                                        fill="none"
                                        stroke="#3ddc84"
                                        stroke-width="1.8"
                                    />
                                </g>
                            }
                        })
                        .collect_view();

                    let axis = ticks
                        .iter()
                        .map(|t| {
                            let x = LABEL_W + (t - start) / per_px;
                            view! {
                                <line
                                    x1=x
                                    y1=AXIS_H - 6.0
                                    x2=x
                                    y2=height
                                    stroke="#1c1f24"
                                    stroke-width="1"
                                />
                                <text
                                    x=x + 3.0
                                    y="13"
                                    font-family="ui-monospace"
                                    font-size="10"
                                    fill="#6b7280"
                                >
                                    {fmt_us(*t)}
                                </text>
                            }
                        })
                        .collect_view();

                    let cursor = hover.get().map(|x| {
                        let at = start + x * per_px;
                        view! {
                            <line
                                x1=LABEL_W + x
                                y1="0"
                                x2=LABEL_W + x
                                y2=height
                                stroke="#e0a838"
                                stroke-width="1"
                                stroke-dasharray="3 3"
                            />
                            <text
                                x=LABEL_W + x + 5.0
                                y=height - 6.0
                                font-family="ui-monospace"
                                font-size="10"
                                fill="#e0a838"
                            >
                                {fmt_us(at)}
                            </text>
                        }
                    });

                    view! {
                        <svg class="h-full w-full" style=format!("min-height: {height}px")>
                            {axis}
                            {lanes}
                            {cursor}
                        </svg>
                    }
                        .into_any()
                }}
            </div>

            {move || {
                let (x, y) = menu.get()?;
                let close = Callback::new(move |_| menu.set(None));
                let follow_label = if follow.get_untracked() {
                    "Stop following"
                } else {
                    "Follow newest edge"
                };
                let no_trace = state.sim_trace.with_untracked(|t| t.events.is_empty());
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            <MenuItem
                                label="Fit the whole capture"
                                disabled=no_trace
                                on_select=Callback::new(move |_| {
                                    fit();
                                    menu.set(None);
                                })
                            />
                            <MenuItem
                                label=follow_label
                                on_select=Callback::new(move |_| {
                                    follow.update(|f| *f = !*f);
                                    menu.set(None);
                                })
                            />
                            <MenuItem
                                label="Export VCD"
                                disabled=no_trace
                                on_select=Callback::new(move |_| {
                                    controller::export_vcd(state);
                                    menu.set(None);
                                })
                            />
                        </ContextMenu>
                    },
                )
            }}
        </div>
    }
}
