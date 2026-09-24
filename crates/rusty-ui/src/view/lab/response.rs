//! The frequency response, measured: a sine stepped across a band on the
//! studied source, each step's gain and phase read off what went in and
//! what came out — beside the design's own curve, so a filter that does not
//! do what it was designed to is seen not to.

use leptos::prelude::*;

use rusty_embed::dsp;
use rusty_i18n::t;

use super::chart::{self, PLAYED};
use super::spectrum::{frequency_ticks, place, record_label, records_offered};
use crate::controller;
use crate::lab::Played;
use crate::lab::record::{self, Of};
use crate::lab::sweep::Point;
use crate::state::{AppState, TraceClock};

/// The design's curve is drawn at this many frequencies across the band.
const CURVE: usize = 240;

/// The rate the firmware prints `channel` at, read off its stamps.
pub(super) fn channel_rate(state: AppState, channel: &str) -> Option<f64> {
    state.sim.plot.with_untracked(|plot| {
        if plot.clock != Some(TraceClock::Firmware) {
            return None;
        }
        let (_, points) = plot.channels.iter().find(|(known, _)| known == channel)?;
        record::from_channel(points).map(|r| r.rate)
    })
}

/// The design's own gain and phase across `from..=to`, at the rate it runs
/// at — or nothing, for a median or a design that cannot be realised.
fn design_curve(design: &dsp::Design, rate: f64, from: f64, to: f64) -> Vec<Point> {
    let Ok(filter) = design.realize(rate) else {
        return Vec::new();
    };
    let top = to.min(rate / 2.0 * 0.999);
    if top <= from {
        return Vec::new();
    }
    let ratio = (top / from).ln() / (CURVE - 1) as f64;
    (0..CURVE)
        .filter_map(|n| {
            let freq = from * (ratio * n as f64).exp();
            let response = filter.response(freq)?;
            Some(Point {
                freq,
                gain: response.gain,
                phase: response.phase,
            })
        })
        .collect()
}

#[component]
pub(super) fn ResponseView(
    tick: RwSignal<u64>,
    played: Memo<Option<Result<Played, String>>>,
) -> impl IntoView {
    let state = AppState::expect();
    let running = move || state.app.session_running.get();
    let sweeping = move || state.lab.sweeping.get().is_some();
    let offered = move || {
        tick.track();
        let has_played = played.with(|p| matches!(p, Some(Ok(_))));
        records_offered(state, has_played)
    };
    let channels = move || {
        offered()
            .into_iter()
            .filter_map(|of| match of {
                Of::Channel(name) => Some(name),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // What goes in is the converter's record while there is one: it is in
    // the counts a firmware's filter reads, so a gain against it is the
    // filter's own and not the converter's scale besides.
    Effect::new(move |_| {
        let first = offered()
            .into_iter()
            .find(|of| matches!(of, Of::Converter(_)));
        if let Some(first) = first
            && state.lab.sweep_input.with_untracked(Option::is_none)
        {
            state.lab.sweep_input.set(Some(first));
        }
    });
    // The first channel the firmware prints is what comes out, until
    // somebody says otherwise.
    Effect::new(move |_| {
        let list = channels();
        if state
            .lab
            .sweep_output
            .with_untracked(|out| out.as_ref().is_none_or(|name| !list.contains(name)))
        {
            state.lab.sweep_output.set(list.first().cloned());
        }
    });

    let field = move |label: String,
                      read: fn(&crate::lab::sweep::Plan) -> f64,
                      write: fn(&mut crate::lab::sweep::Plan, f64)| {
        view! {
            <label class="flex items-center gap-1 text-caption text-label-3">
                <span>{label}</span>
                <input
                    type="text"
                    prop:value=move || chart::tick(state.lab.sweep.with(read))
                    on:change=move |event| {
                        if let Ok(value) = event_target_value(&event).trim().parse::<f64>() {
                            state.lab.sweep.update(|plan| write(plan, value));
                        }
                    }
                    class="h-[22px] w-[4.5rem] rounded-[5px] bg-sunken px-1.5 font-mono text-caption text-label outline-none ring-1 ring-line focus:ring-rust"
                />
            </label>
        }
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 pt-1.5">
                {field(t!("lab.sweep-from"), |p| p.from, |p, v| p.from = v)}
                {field(t!("lab.sweep-to"), |p| p.to, |p, v| p.to = v)}
                {field(t!("lab.sweep-points"), |p| p.points as f64, |p, v| p.points = v.clamp(1.0, 200.0) as usize)}
                {field(t!("lab.sweep-amplitude"), |p| p.amplitude, |p, v| p.amplitude = v)}
                {field(t!("lab.sweep-offset"), |p| p.offset, |p, v| p.offset = v)}
            </div>
            <div class="flex flex-wrap items-center gap-2 px-3 pt-1">
                <label class="flex items-center gap-1 text-caption text-label-3" title=t!("lab.sweep-input-hint")>
                    <span>{t!("lab.sweep-input")}</span>
                    <select
                        on:change=move |event| {
                            let value = event_target_value(&event);
                            let of = offered().into_iter().find(|of| format!("{of:?}") == value);
                            state.lab.sweep_input.set(of);
                        }
                        class="h-[22px] max-w-[10rem] rounded-[5px] bg-sunken px-1 text-caption text-label-2 outline-none"
                    >
                        {move || played.with(|p| {
                            let played = p.as_ref().and_then(|p| p.as_ref().ok());
                            let chosen = state.lab.sweep_input.get().unwrap_or(Of::Played);
                            offered()
                                .into_iter()
                                .map(|of| {
                                    let label = record_label(&of, played);
                                    let key = format!("{of:?}");
                                    let selected = of == chosen;
                                    view! { <option value=key selected=selected>{label}</option> }
                                })
                                .collect_view()
                        })}
                    </select>
                </label>
                <label class="flex items-center gap-1 text-caption text-label-3" title=t!("lab.sweep-output-hint")>
                    <span>{t!("lab.sweep-output")}</span>
                    <select
                        on:change=move |event| state.lab.sweep_output.set(Some(event_target_value(&event)))
                        class="h-[22px] max-w-[10rem] rounded-[5px] bg-sunken px-1 text-caption text-label-2 outline-none"
                    >
                        {move || {
                            let chosen = state.lab.sweep_output.get();
                            channels()
                                .into_iter()
                                .map(|name| {
                                    let selected = chosen.as_deref() == Some(name.as_str());
                                    view! { <option value=name.clone() selected=selected>{name.clone()}</option> }
                                })
                                .collect_view()
                        }}
                    </select>
                </label>
                <span class="flex-1" />
                {move || {
                    if sweeping() {
                        view! {
                            <button
                                type="button"
                                on:click=move |_| controller::stop_sweep(state)
                                class="rounded-[5px] px-2 py-0.5 text-footnote text-crimson hover:bg-sunken"
                            >
                                {t!("lab.sweep-stop")}
                            </button>
                        }
                        .into_any()
                    } else {
                        let ready = running() && state.lab.sweep_output.with(Option::is_some);
                        view! {
                            <button
                                type="button"
                                disabled=!ready
                                title=t!("lab.sweep-start-hint")
                                on:click=move |_| controller::start_sweep(state)
                                class="rounded-[5px] bg-rust px-2 py-0.5 text-footnote text-white hover:opacity-90 disabled:opacity-40"
                            >
                                {t!("lab.sweep-start")}
                            </button>
                        }
                        .into_any()
                    }
                }}
            </div>
            <p class="px-3 pt-1 text-caption text-label-4">
                {move || {
                    if let Some((step, of, freq)) = state.lab.sweep_step.get() {
                        t!("lab.sweep-step", step = step, of = of, freq = chart::tick(freq))
                    } else if let Some(why) = state.lab.sweep_missed.get() {
                        why
                    } else if !running() {
                        t!("lab.sweep-needs-run")
                    } else if channels().is_empty() {
                        t!("lab.sweep-needs-output")
                    } else {
                        t!("lab.sweep-ready")
                    }
                }}
            </p>
            {move || {
                let measured = state.lab.swept.get();
                let plan = state.lab.sweep.get();
                let rate = state.lab.design_rate.get().or_else(|| {
                    state.lab.sweep_output.get().and_then(|name| channel_rate(state, &name))
                });
                let curve = rate
                    .map(|rate| state.lab.design.with(|d| design_curve(d, rate, plan.from / 1.25, plan.to * 1.25)))
                    .unwrap_or_default();
                view! { <ResponseChart measured=measured curve=curve from=plan.from / 1.25 to=plan.to * 1.25 /> }
            }}
        </div>
    }
}

#[component]
fn ResponseChart(measured: Vec<Point>, curve: Vec<Point>, from: f64, to: f64) -> impl IntoView {
    const W: f64 = 1000.0;
    const H: f64 = 100.0;
    let db = |p: &Point| dsp::db(p.gain).max(-120.0);
    let degrees = |p: &Point| p.phase.to_degrees();
    let gains = measured
        .iter()
        .chain(&curve)
        .map(db)
        .filter(|v| v.is_finite());
    let (low, high) = chart::band(gains).unwrap_or((-40.0, 5.0));
    let (low, high) = ((low - 3.0).max(-120.0), (high + 3.0).min(40.0));
    let at = |freq: f64| place(freq, from, to, true);
    let line = |points: &[Point], value: &dyn Fn(&Point) -> f64, range: (f64, f64)| {
        let placed: Vec<(f64, f64)> = points.iter().map(|p| (at(p.freq), value(p))).collect();
        chart::path(&placed, range, W, H)
    };
    let gain_curve = line(&curve, &db, (low, high));
    let phase_curve = line(&curve, &degrees, (-180.0, 180.0));
    let gain_measured = line(&measured, &db, (low, high));
    let phase_measured = line(&measured, &degrees, (-180.0, 180.0));
    let dots = |value: &dyn Fn(&Point) -> f64, range: (f64, f64)| {
        measured
            .iter()
            .map(|p| {
                let x = at(p.freq) * W;
                let y = H - 2.0 - (value(p) - range.0) / (range.1 - range.0).max(1e-12) * (H - 4.0);
                view! { <circle cx=x cy=y r="2.5" fill="#e0a838" /> }
            })
            .collect_view()
    };
    let ticks = frequency_ticks(from, to, true);
    let grid = move || {
        ticks
            .iter()
            .map(|freq| {
                let x = at(*freq) * W;
                view! { <line x1=x y1="0" x2=x y2=H stroke="#3c434e" stroke-width="1" vector-effect="non-scaling-stroke" /> }
            })
            .collect_view()
    };
    let labels = frequency_ticks(from, to, true);
    let empty = measured.is_empty() && curve.is_empty();

    view! {
        <div class="flex min-h-0 flex-1 flex-col gap-px px-3 pb-2 pt-1">
            {empty.then(|| view! {
                <p class="text-caption text-label-3">{t!("lab.response-empty")}</p>
            })}
            <div class="relative min-h-[48px] flex-1 rounded-[4px] bg-sunken/40">
                <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" class="absolute inset-0 h-full w-full">
                    {grid()}
                    <path d=gain_curve fill="none" stroke=PLAYED stroke-width="1.2" stroke-dasharray="5 4" vector-effect="non-scaling-stroke" />
                    <path d=gain_measured fill="none" stroke="#e0a838" stroke-width="1.4" vector-effect="non-scaling-stroke" />
                </svg>
                <svg viewBox=format!("0 0 {W} {H}") class="absolute inset-0 h-full w-full" preserveAspectRatio="none">
                    {dots(&db, (low, high))}
                </svg>
                <span class="pointer-events-none absolute left-1.5 top-0.5 font-mono text-caption text-label-3">
                    {t!("lab.gain")}
                </span>
                <span class="pointer-events-none absolute right-1.5 top-0.5 font-mono text-caption text-label-4">
                    {format!("{low:.0} … {high:.0} dB")}
                </span>
            </div>
            <div class="relative min-h-[48px] flex-1 rounded-[4px] bg-sunken/40">
                <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" class="absolute inset-0 h-full w-full">
                    <line x1="0" y1={H / 2.0} x2=W y2={H / 2.0} stroke="#3c434e" stroke-width="1" vector-effect="non-scaling-stroke" />
                    <path d=phase_curve fill="none" stroke=PLAYED stroke-width="1.2" stroke-dasharray="5 4" vector-effect="non-scaling-stroke" />
                    <path d=phase_measured fill="none" stroke="#e0a838" stroke-width="1.4" vector-effect="non-scaling-stroke" />
                </svg>
                <svg viewBox=format!("0 0 {W} {H}") class="absolute inset-0 h-full w-full" preserveAspectRatio="none">
                    {dots(&degrees, (-180.0, 180.0))}
                </svg>
                <span class="pointer-events-none absolute left-1.5 top-0.5 font-mono text-caption text-label-3">
                    {t!("lab.phase")}
                </span>
                <span class="pointer-events-none absolute right-1.5 top-0.5 font-mono text-caption text-label-4">
                    "−180° … 180°"
                </span>
            </div>
            <div class="relative h-4">
                {labels.iter().map(|freq| {
                    let left = at(*freq) * 100.0;
                    view! {
                        <span class="absolute -translate-x-1/2 font-mono text-caption text-label-4" style=format!("left: {left}%")>
                            {chart::tick(*freq)}
                        </span>
                    }
                }).collect_view()}
            </div>
            <p class="flex gap-3 font-mono text-caption text-label-4">
                <span><span class="text-[#e0a838]">"● "</span>{t!("lab.measured")}</span>
                <span><span style=format!("color: {PLAYED}")>"- - "</span>{t!("lab.designed")}</span>
            </p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::dsp::{Design, Pass};

    /// The design's curve is its own response: a Butterworth low-pass reads
    /// −3 dB at its cutoff, and a curve asked for past half the rate stops
    /// there.
    #[test]
    fn the_designed_curve_is_the_filters_own() {
        let design = Design::Butterworth {
            pass: Pass::Low,
            order: 2,
            cutoff: 10.0,
        };
        let curve = design_curve(&design, 1000.0, 1.0, 100.0);
        let at_cutoff = curve
            .iter()
            .min_by(|a, b| (a.freq - 10.0).abs().total_cmp(&(b.freq - 10.0).abs()))
            .unwrap();
        assert!((dsp::db(at_cutoff.gain) + 3.0).abs() < 0.2, "{at_cutoff:?}");
        let past = design_curve(&design, 100.0, 1.0, 1000.0);
        assert!(past.iter().all(|p| p.freq < 50.0));
        assert!(design_curve(&Design::Median { taps: 5 }, 1000.0, 1.0, 100.0).is_empty());
    }
}
