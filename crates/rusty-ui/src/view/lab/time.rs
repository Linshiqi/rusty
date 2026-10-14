//! The signal, what the converter took, and what the firmware printed, as
//! lanes on one clock — the firmware's own.
//!
//! Lanes and not one axis: a signal in volts, a converter in counts and a
//! filter's output in whatever the firmware prints are three units, and
//! drawn against one scale two of them would be flat lines. What lines them
//! up is time, which all three carry from the same clock: the emulator's
//! account of when a table started, its stamp on every conversion, and the
//! firmware's stamp on every line it prints.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::lab::{Played, start_of};
use crate::state::{AppState, TraceClock};
use rusty_embed::lab::chart::{self, INK, PLAYED};
use rusty_embed::lab::time::{conversion_steps, played_points};

/// The windows offered, in seconds.
const WINDOWS: [f64; 5] = [0.05, 0.2, 1.0, 5.0, 20.0];

/// Columns a lane is thinned to: more than a dock is wide in pixels.
const COLUMNS: usize = 800;

/// One lane: what it is, its colour, and its points as `(x from 0 to 1, y)`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Lane {
    pub label: String,
    pub ink: &'static str,
    pub points: Vec<(f64, f64)>,
}

/// Times into the window's width.
fn across(points: Vec<(u64, f64)>, from: u64, to: u64) -> Vec<(f64, f64)> {
    let span = (to - from).max(1) as f64;
    let placed: Vec<(f64, f64)> = points
        .into_iter()
        .map(|(at, value)| ((at.saturating_sub(from)) as f64 / span, value))
        .collect();
    chart::thin(&placed, COLUMNS)
}

/// The newest instant anything on the firmware's clock has said: a
/// conversion, or a stamped line of telemetry.
fn latest(state: AppState) -> Option<u64> {
    let converted = state.lab.conversions.with_untracked(|all| {
        all.values()
            .filter_map(|record| record.last().map(|(at, _)| *at))
            .max()
    });
    let printed = state.sim.plot.with_untracked(|plot| {
        (plot.clock == Some(TraceClock::Firmware))
            .then(|| {
                plot.channels
                    .iter()
                    .filter_map(|(_, points)| points.last().map(|(at, _)| *at))
                    .max()
            })
            .flatten()
    });
    converted.max(printed)
}

/// Every lane for the window ending at the newest instant: what was played,
/// each converter's record, and each channel the firmware prints.
fn lanes(state: AppState, played: Option<&Played>, window_us: u64) -> Option<Vec<Lane>> {
    let to = latest(state)?;
    let from = to.saturating_sub(window_us);
    let mut lanes = Vec::new();

    if let Some(played) = played {
        let start = state
            .lab
            .switched
            .with_untracked(|switched| start_of(played, switched));
        if let Some(start) = start {
            lanes.push(Lane {
                label: t!("lab.lane-played", source = played.source.label()),
                ink: PLAYED,
                points: across(played_points(played, start, from, to), from, to),
            });
        }
    }
    state.lab.conversions.with_untracked(|all| {
        for (pin, record) in all {
            let steps = conversion_steps(record, from, to);
            if steps.is_empty() {
                continue;
            }
            lanes.push(Lane {
                label: t!("lab.lane-converter", pin = *pin),
                ink: "#9aa2ae",
                points: across(steps, from, to),
            });
        }
    });
    state.sim.plot.with_untracked(|plot| {
        if plot.clock != Some(TraceClock::Firmware) {
            return;
        }
        for (index, (name, points)) in plot.channels.iter().enumerate() {
            let begin = points.partition_point(|(at, _)| *at < from);
            let inside: Vec<(u64, f64)> = points[begin..]
                .iter()
                .take_while(|(at, _)| *at <= to)
                .map(|(at, value)| (*at, f64::from(*value)))
                .collect();
            if inside.is_empty() {
                continue;
            }
            lanes.push(Lane {
                label: name.clone(),
                ink: INK[index % INK.len()],
                points: across(inside, from, to),
            });
        }
    });
    Some(lanes)
}

#[component]
pub(super) fn TimeView(
    tick: RwSignal<u64>,
    played: Memo<Option<Result<Played, String>>>,
) -> impl IntoView {
    let state = AppState::expect();
    let window = RwSignal::new(1.0f64);
    let drawn = move || {
        tick.track();
        let window_us = (window.get() * 1e6) as u64;
        played.with(|played| {
            let played = played.as_ref().and_then(|p| p.as_ref().ok());
            lanes(state, played, window_us)
        })
    };
    let host_clock = move || {
        tick.track();
        state.sim.plot.with_untracked(|plot| {
            plot.clock == Some(TraceClock::Host) && !plot.channels.is_empty()
        })
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex items-center gap-1 px-3 pt-1.5">
                <span class="text-caption text-label-4" title=t!("lab.window-hint")>{t!("lab.window")}</span>
                {WINDOWS
                    .into_iter()
                    .map(|seconds| {
                        view! {
                            <button
                                type="button"
                                on:click=move |_| window.set(seconds)
                                class=move || {
                                    let base = "rounded-[5px] px-1.5 py-0.5 font-mono text-caption";
                                    if window.get() == seconds {
                                        format!("{base} bg-sunken text-label")
                                    } else {
                                        format!("{base} text-label-3 hover:text-label")
                                    }
                                }
                            >
                                {format!("{} s", chart::tick(seconds))}
                            </button>
                        }
                    })
                    .collect_view()}
                <span class="flex-1" />
                {move || host_clock().then(|| view! {
                    <span class="text-caption text-amber">{t!("lab.host-clock")}</span>
                })}
            </div>
            {move || match drawn() {
                None => view! {
                    <p class="px-3 py-2 text-caption leading-relaxed text-label-3">{t!("lab.time-empty")}</p>
                }
                .into_any(),
                Some(lanes) if lanes.is_empty() => view! {
                    <p class="px-3 py-2 text-caption leading-relaxed text-label-3">{t!("lab.time-empty")}</p>
                }
                .into_any(),
                Some(lanes) => view! {
                    <div class="flex min-h-0 flex-1 flex-col gap-px px-3 pb-2 pt-1">
                        {lanes.into_iter().map(|lane| view! { <LaneView lane=lane /> }).collect_view()}
                    </div>
                }
                .into_any(),
            }}
        </div>
    }
}

#[component]
fn LaneView(lane: Lane) -> impl IntoView {
    const W: f64 = 1000.0;
    const H: f64 = 100.0;
    let range = chart::band(lane.points.iter().map(|(_, y)| *y)).unwrap_or((0.0, 1.0));
    let drawn = chart::path(&lane.points, range, W, H);
    let scale = format!("{} … {}", chart::tick(range.0), chart::tick(range.1));
    view! {
        <div class="relative min-h-[36px] flex-1 rounded-[4px] bg-sunken/40">
            <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" class="absolute inset-0 h-full w-full">
                <path d=drawn fill="none" stroke=lane.ink stroke-width="1.4" vector-effect="non-scaling-stroke" />
            </svg>
            <span class="pointer-events-none absolute left-1.5 top-0.5 font-mono text-caption" style=format!("color: {}", lane.ink)>
                {lane.label}
            </span>
            <span class="pointer-events-none absolute right-1.5 top-0.5 font-mono text-caption text-label-4">
                {scale}
            </span>
        </div>
    }
}
