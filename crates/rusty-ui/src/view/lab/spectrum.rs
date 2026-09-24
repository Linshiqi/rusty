//! Any record as a spectrum: what is in the signal, what the converter
//! took, and what the firmware's filter let through, in decibels against
//! frequency, with the source's own tones marked.

use leptos::prelude::*;

use rusty_embed::dsp::{self, Window};
use rusty_embed::signal::Component;
use rusty_i18n::t;

use super::chart::{self, INK, PLAYED};
use crate::lab::record::{self, Of, Record};
use crate::lab::{Played, start_of};
use crate::state::{AppState, TraceClock};

/// The most samples a spectrum is taken over: a power of two, and as much
/// as the one thread there is can transform four times a second.
const MOST: usize = 1 << 16;

/// How far below the highest bin the chart reaches.
const DEPTH_DB: f64 = 100.0;

/// The records there are to read: what is played, each converter that has
/// reported, and each channel the firmware prints on its own clock.
pub(super) fn records_offered(state: AppState, played: bool) -> Vec<Of> {
    let mut offered = Vec::new();
    if played {
        offered.push(Of::Played);
    }
    state.lab.conversions.with_untracked(|all| {
        offered.extend(all.keys().map(|pin| Of::Converter(*pin)));
    });
    state.sim.plot.with_untracked(|plot| {
        if plot.clock == Some(TraceClock::Firmware) {
            offered.extend(
                plot.channels
                    .iter()
                    .map(|(name, _)| Of::Channel(name.clone())),
            );
        }
    });
    offered
}

/// What a record is called on a chip.
pub(super) fn record_label(of: &Of, played: Option<&Played>) -> String {
    match of {
        Of::Played => played.map_or_else(String::new, |p| {
            t!("lab.lane-played", source = p.source.label())
        }),
        Of::Converter(pin) => t!("lab.lane-converter", pin = *pin),
        Of::Channel(name) => name.clone(),
    }
}

/// The last `seconds` of a record, as it stands now.
pub(super) fn take(
    state: AppState,
    of: &Of,
    played: Option<&Played>,
    seconds: f64,
) -> Option<Record> {
    let whole = match of {
        Of::Played => {
            let played = played?;
            let start = state
                .lab
                .switched
                .with_untracked(|switched| start_of(played, switched))
                .unwrap_or(0);
            // Playing: the last span of it on the emulator's clock. Not
            // playing: from its first sample, which is what it will be.
            let end = state
                .lab
                .conversions
                .with_untracked(|all| {
                    all.values()
                        .filter_map(|r| r.last().map(|(at, _)| *at))
                        .max()
                })
                .unwrap_or(start + (seconds * 1e6) as u64);
            let from = end.saturating_sub((seconds * 1e6) as u64).max(start);
            record::from_played(played, start, from, f64::from(played.rate), seconds)
        }
        Of::Converter(pin) => state
            .lab
            .conversions
            .with_untracked(|all| all.get(pin).and_then(|r| record::from_conversions(r)))?,
        Of::Channel(name) => state.sim.plot.with_untracked(|plot| {
            let (_, points) = plot.channels.iter().find(|(known, _)| known == name)?;
            record::from_channel(points)
        })?,
    };
    let keep = ((seconds * whole.rate) as usize).min(MOST);
    let skip = whole.samples.len().saturating_sub(keep);
    Some(Record {
        rate: whole.rate,
        start_us: whole.start_us + (skip as f64 * 1e6 / whole.rate) as u64,
        samples: whole.samples[skip..].to_vec(),
    })
}

/// The frequencies a signal is made of, where it has any to mark: each
/// tone's fundamental. Noise, steps and sweeps have no line of their own.
pub(super) fn tones_of(played: Option<&Played>) -> Vec<f64> {
    let Some(played) = played else {
        return Vec::new();
    };
    played
        .signal
        .components
        .iter()
        .filter_map(|component| match *component {
            Component::Sine { freq, .. }
            | Component::Square { freq, .. }
            | Component::Triangle { freq, .. }
            | Component::Sawtooth { freq, .. } => Some(freq),
            _ => None,
        })
        .collect()
}

#[component]
pub(super) fn SpectrumView(
    tick: RwSignal<u64>,
    played: Memo<Option<Result<Played, String>>>,
) -> impl IntoView {
    let state = AppState::expect();
    let chosen = RwSignal::new(None::<Of>);
    let seconds = RwSignal::new(5.0f64);
    let window = RwSignal::new(Window::Hann);
    // Logarithmic: a filter is read in octaves and decades, and on a
    // straight axis every tone below a tenth of the band shares one pixel.
    let log_axis = RwSignal::new(true);
    // A spectrum a second is plenty to watch a filter by, and a transform
    // four times a second is work the chart cannot show.
    let slow = Memo::new(move |_| tick.get() / 4);

    let offered = move || {
        slow.track();
        let has_played = played.with(|p| matches!(p, Some(Ok(_))));
        records_offered(state, has_played)
    };
    let current = move || {
        let offered = offered();
        chosen
            .get()
            .filter(|of| offered.contains(of))
            .or_else(|| offered.first().cloned())
    };
    let computed = move || {
        slow.track();
        let of = current()?;
        played.with(|p| {
            let played = p.as_ref().and_then(|p| p.as_ref().ok());
            let record = take(state, &of, played, seconds.get())?;
            let found = dsp::spectrum(&record.samples, record.rate, window.get());
            (!found.freqs.is_empty()).then_some((
                found,
                record.rate,
                record.samples.len(),
                tones_of(played),
            ))
        })
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex flex-wrap items-center gap-1 px-3 pt-1.5">
                {move || {
                    let now = current();
                    played.with(|p| {
                        let played = p.as_ref().and_then(|p| p.as_ref().ok());
                        offered()
                            .into_iter()
                            .enumerate()
                            .map(|(index, of)| {
                                let label = record_label(&of, played);
                                let on = now.as_ref() == Some(&of);
                                let ink = if of == Of::Played { PLAYED } else { INK[index % INK.len()] };
                                let pick = of.clone();
                                view! {
                                    <button
                                        type="button"
                                        on:click=move |_| chosen.set(Some(pick.clone()))
                                        class=if on {
                                            "rounded-[5px] bg-sunken px-1.5 py-0.5 font-mono text-caption text-label"
                                        } else {
                                            "rounded-[5px] px-1.5 py-0.5 font-mono text-caption text-label-3 hover:text-label"
                                        }
                                    >
                                        <span style=format!("color: {ink}")>"— "</span>
                                        {label}
                                    </button>
                                }
                            })
                            .collect_view()
                    })
                }}
                <span class="flex-1" />
                <select
                    title=t!("lab.span-hint")
                    on:change=move |event| {
                        if let Ok(value) = event_target_value(&event).parse::<f64>() {
                            seconds.set(value);
                        }
                    }
                    class="h-[22px] rounded-[5px] bg-sunken px-1 text-caption text-label-2 outline-none"
                >
                    {[1.0, 5.0, 20.0]
                        .into_iter()
                        .map(|value| view! {
                            <option value=value.to_string() selected=move || seconds.get() == value>
                                {format!("{} s", chart::tick(value))}
                            </option>
                        })
                        .collect_view()}
                </select>
                <select
                    title=t!("lab.window-kind-hint")
                    on:change=move |event| {
                        window.set(match event_target_value(&event).as_str() {
                            "blackman" => Window::Blackman,
                            "rectangular" => Window::Rectangular,
                            _ => Window::Hann,
                        })
                    }
                    class="h-[22px] rounded-[5px] bg-sunken px-1 text-caption text-label-2 outline-none"
                >
                    <option value="hann">"Hann"</option>
                    <option value="blackman">"Blackman"</option>
                    <option value="rectangular">{t!("lab.window-rectangular")}</option>
                </select>
                <button
                    type="button"
                    on:click=move |_| log_axis.update(|log| *log = !*log)
                    class=move || if log_axis.get() {
                        "rounded-[5px] bg-sunken px-1.5 py-0.5 text-caption text-label"
                    } else {
                        "rounded-[5px] px-1.5 py-0.5 text-caption text-label-3 hover:text-label"
                    }
                >
                    {t!("lab.log-axis")}
                </button>
            </div>
            {move || match computed() {
                None => view! {
                    <p class="px-3 py-2 text-caption leading-relaxed text-label-3">{t!("lab.spectrum-empty")}</p>
                }
                .into_any(),
                Some((found, rate, count, tones)) => view! {
                    <SpectrumChart found=found rate=rate count=count tones=tones log=log_axis.get() />
                }
                .into_any(),
            }}
        </div>
    }
}

/// Where `freq` sits across a chart from `low` to `high` hertz.
pub(super) fn place(freq: f64, low: f64, high: f64, log: bool) -> f64 {
    if log {
        let (low, high) = (low.max(1e-9).ln(), high.max(1e-9).ln());
        (freq.max(1e-9).ln() - low) / (high - low).max(1e-12)
    } else {
        (freq - low) / (high - low).max(1e-12)
    }
}

/// Round numbers along a frequency axis: every decade's 1, 2 and 5 on a
/// logarithmic one, five even steps on a straight one.
pub(super) fn frequency_ticks(low: f64, high: f64, log: bool) -> Vec<f64> {
    if log {
        let mut ticks = Vec::new();
        let mut decade = 10f64.powf(low.max(1e-6).log10().floor());
        while decade <= high {
            for step in [1.0, 2.0, 5.0] {
                let at = decade * step;
                if at >= low && at <= high {
                    ticks.push(at);
                }
            }
            decade *= 10.0;
        }
        ticks
    } else {
        let step = nice_step((high - low) / 5.0);
        let mut at = (low / step).ceil() * step;
        let mut ticks = Vec::new();
        while at <= high + step * 1e-9 {
            ticks.push(at);
            at += step;
        }
        ticks
    }
}

/// 1, 2 or 5 times a power of ten, the nearest at or above `rough`.
fn nice_step(rough: f64) -> f64 {
    let power = 10f64.powf(rough.max(1e-12).log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * power)
        .find(|step| *step >= rough)
        .unwrap_or(10.0 * power)
}

#[component]
fn SpectrumChart(
    found: dsp::Spectrum,
    rate: f64,
    count: usize,
    tones: Vec<f64>,
    log: bool,
) -> impl IntoView {
    const W: f64 = 1000.0;
    const H: f64 = 200.0;
    let nyquist = rate / 2.0;
    let low = if log {
        found.freqs.get(1).copied().unwrap_or(1.0)
    } else {
        0.0
    };
    let decibels: Vec<f64> = found.amplitude.iter().map(|a| dsp::db(*a)).collect();
    let top = decibels
        .iter()
        .copied()
        .filter(|d| d.is_finite())
        .fold(f64::MIN, f64::max);
    let floor = top - DEPTH_DB;
    let points: Vec<(f64, f64)> = found
        .freqs
        .iter()
        .zip(&decibels)
        .filter(|(freq, _)| !log || **freq >= low)
        .map(|(freq, db)| (place(*freq, low, nyquist, log), db.max(floor)))
        .collect();
    let thinned = chart::thin(&points, 900);
    let path = chart::path(&thinned, (floor, top), W, H);
    // The highest bin, which is what a reader looks for first — past the
    // three a window smears an offset into, or every signal with a DC level
    // would say its loudest tone is at zero.
    let peak = found
        .freqs
        .iter()
        .zip(&decibels)
        .skip(3)
        .fold(
            (0.0, f64::MIN),
            |best, (f, d)| if *d > best.1 { (*f, *d) } else { best },
        );
    let ticks = frequency_ticks(low, nyquist, log);
    let caption = t!(
        "lab.spectrum-caption",
        count = count,
        rate = chart::tick(rate),
        peak = chart::tick(peak.0),
        level = format!("{:.1}", peak.1)
    );

    view! {
        <div class="flex min-h-0 flex-1 flex-col px-3 pb-2 pt-1">
            <div class="relative min-h-[80px] flex-1 rounded-[4px] bg-sunken/40">
                <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" class="absolute inset-0 h-full w-full">
                    {ticks.iter().map(|freq| {
                        let x = place(*freq, low, nyquist, log) * W;
                        view! { <line x1=x y1="0" x2=x y2=H stroke="#3c434e" stroke-width="1" vector-effect="non-scaling-stroke" /> }
                    }).collect_view()}
                    {tones.iter().filter(|f| **f > low && **f < nyquist).map(|freq| {
                        let x = place(*freq, low, nyquist, log) * W;
                        view! { <line x1=x y1="0" x2=x y2=H stroke=PLAYED stroke-width="1" stroke-dasharray="4 4" vector-effect="non-scaling-stroke" /> }
                    }).collect_view()}
                    <path d=path fill="none" stroke="#e0a838" stroke-width="1.3" vector-effect="non-scaling-stroke" />
                </svg>
                <span class="pointer-events-none absolute right-1.5 top-0.5 font-mono text-caption text-label-4">
                    {format!("{:.0} … {:.0} dB", floor, top)}
                </span>
            </div>
            <div class="relative h-4">
                {ticks.iter().map(|freq| {
                    let left = place(*freq, low, nyquist, log) * 100.0;
                    view! {
                        <span class="absolute -translate-x-1/2 font-mono text-caption text-label-4" style=format!("left: {left}%")>
                            {chart::tick(*freq)}
                        </span>
                    }
                }).collect_view()}
            </div>
            <p class="font-mono text-caption text-label-4">{caption}</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frequency_is_placed_on_either_axis() {
        assert!((place(250.0, 0.0, 500.0, false) - 0.5).abs() < 1e-12);
        assert!((place(10.0, 1.0, 100.0, true) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn ticks_are_round_numbers() {
        assert_eq!(
            frequency_ticks(0.0, 500.0, false),
            [0.0, 100.0, 200.0, 300.0, 400.0, 500.0]
        );
        assert_eq!(
            frequency_ticks(1.0, 100.0, true),
            [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]
        );
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(0.07), 0.1);
    }
}
