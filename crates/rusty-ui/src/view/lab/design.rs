//! A filter chosen and tuned against the signal without running anything:
//! its response over the signal's own spectrum, what it makes of the signal
//! in time, and its code to copy into the firmware — which computes, in
//! `f32`, what the charts show (`rusty_embed::dsp`, `code`).

use leptos::prelude::*;

use rusty_embed::dsp::{self, Band, BiquadKind, Design, Pass, Window};
use rusty_i18n::t;

use super::chart::{self, PLAYED};
use super::response::channel_rate;
use super::spectrum::{frequency_ticks, place};
use crate::lab::record;
use crate::lab::{Played, dsp_refusal_text};
use crate::state::{AppState, TraceClock};

/// The kinds of filter offered, in the order a person reaches for them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    MovingAverage,
    Exponential,
    ButterLow,
    ButterHigh,
    LowBiquad,
    HighBiquad,
    BandBiquad,
    Notch,
    FirLow,
    FirHigh,
    FirBand,
    Median,
}

impl Kind {
    pub(super) const ALL: [Kind; 12] = [
        Kind::MovingAverage,
        Kind::Exponential,
        Kind::ButterLow,
        Kind::ButterHigh,
        Kind::LowBiquad,
        Kind::HighBiquad,
        Kind::BandBiquad,
        Kind::Notch,
        Kind::FirLow,
        Kind::FirHigh,
        Kind::FirBand,
        Kind::Median,
    ];

    pub(super) fn of(design: &Design) -> Kind {
        match design {
            Design::MovingAverage { .. } => Kind::MovingAverage,
            Design::Exponential { .. } => Kind::Exponential,
            Design::Butterworth {
                pass: Pass::Low, ..
            } => Kind::ButterLow,
            Design::Butterworth {
                pass: Pass::High, ..
            } => Kind::ButterHigh,
            Design::Biquad { kind, .. } => match kind {
                BiquadKind::LowPass => Kind::LowBiquad,
                BiquadKind::HighPass => Kind::HighBiquad,
                BiquadKind::BandPass => Kind::BandBiquad,
                BiquadKind::Notch => Kind::Notch,
            },
            Design::Fir { band, .. } => match band {
                Band::LowPass { .. } => Kind::FirLow,
                Band::HighPass { .. } => Kind::FirHigh,
                Band::BandPass { .. } => Kind::FirBand,
            },
            Design::Median { .. } => Kind::Median,
        }
    }

    fn label(self) -> String {
        match self {
            Kind::MovingAverage => t!("lab.kind-moving-average"),
            Kind::Exponential => t!("lab.kind-exponential"),
            Kind::ButterLow => t!("lab.kind-butterworth-low"),
            Kind::ButterHigh => t!("lab.kind-butterworth-high"),
            Kind::LowBiquad => t!("lab.kind-biquad-low"),
            Kind::HighBiquad => t!("lab.kind-biquad-high"),
            Kind::BandBiquad => t!("lab.kind-biquad-band"),
            Kind::Notch => t!("lab.kind-notch"),
            Kind::FirLow => t!("lab.kind-fir-low"),
            Kind::FirHigh => t!("lab.kind-fir-high"),
            Kind::FirBand => t!("lab.kind-fir-band"),
            Kind::Median => t!("lab.kind-median"),
        }
    }

    /// A design of this kind, keeping what the last one said where it can
    /// mean the same — its corner, its taps — so switching kinds compares
    /// two filters at one frequency rather than starting from nothing.
    pub(super) fn design(self, from: &Design) -> Design {
        let corner = corner_of(from).unwrap_or(10.0);
        let taps = taps_of(from).unwrap_or(31);
        let odd = taps | 1;
        match self {
            Kind::MovingAverage => Design::MovingAverage { taps: taps.min(64) },
            Kind::Exponential => Design::Exponential { alpha: 0.1 },
            Kind::ButterLow | Kind::ButterHigh => Design::Butterworth {
                pass: if self == Kind::ButterLow {
                    Pass::Low
                } else {
                    Pass::High
                },
                order: 2,
                cutoff: corner,
            },
            Kind::LowBiquad | Kind::HighBiquad | Kind::BandBiquad | Kind::Notch => Design::Biquad {
                kind: match self {
                    Kind::LowBiquad => BiquadKind::LowPass,
                    Kind::HighBiquad => BiquadKind::HighPass,
                    Kind::BandBiquad => BiquadKind::BandPass,
                    _ => BiquadKind::Notch,
                },
                cutoff: corner,
                q: std::f64::consts::FRAC_1_SQRT_2,
            },
            Kind::FirLow => Design::Fir {
                band: Band::LowPass { cutoff: corner },
                taps,
                window: Window::Hann,
            },
            Kind::FirHigh => Design::Fir {
                band: Band::HighPass { cutoff: corner },
                taps: odd,
                window: Window::Hann,
            },
            Kind::FirBand => Design::Fir {
                band: Band::BandPass {
                    low: corner / 2.0,
                    high: corner * 2.0,
                },
                taps,
                window: Window::Hann,
            },
            Kind::Median => Design::Median { taps: 5 },
        }
    }
}

fn corner_of(design: &Design) -> Option<f64> {
    match *design {
        Design::Biquad { cutoff, .. } | Design::Butterworth { cutoff, .. } => Some(cutoff),
        Design::Fir { band, .. } => Some(match band {
            Band::LowPass { cutoff } | Band::HighPass { cutoff } => cutoff,
            Band::BandPass { low, high } => (low * high).sqrt(),
        }),
        _ => None,
    }
}

fn taps_of(design: &Design) -> Option<usize> {
    match *design {
        Design::MovingAverage { taps } | Design::Fir { taps, .. } | Design::Median { taps } => {
            Some(taps)
        }
        _ => None,
    }
}

/// One number a design is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Param {
    Taps,
    Alpha,
    Cutoff,
    Q,
    Order,
    Low,
    High,
}

impl Param {
    fn label(self) -> String {
        match self {
            Param::Taps => t!("lab.param-taps"),
            Param::Alpha => t!("lab.param-alpha"),
            Param::Cutoff => t!("lab.param-cutoff"),
            Param::Q => t!("lab.param-q"),
            Param::Order => t!("lab.param-order"),
            Param::Low => t!("lab.param-low"),
            Param::High => t!("lab.param-high"),
        }
    }
}

/// The numbers `design` is made of, in the order they are asked for.
pub(super) fn params(design: &Design) -> Vec<(Param, f64)> {
    match *design {
        Design::MovingAverage { taps } | Design::Median { taps } => {
            vec![(Param::Taps, taps as f64)]
        }
        Design::Exponential { alpha } => vec![(Param::Alpha, alpha)],
        Design::Biquad { cutoff, q, .. } => vec![(Param::Cutoff, cutoff), (Param::Q, q)],
        Design::Butterworth { order, cutoff, .. } => {
            vec![(Param::Order, order as f64), (Param::Cutoff, cutoff)]
        }
        Design::Fir { band, taps, .. } => {
            let mut out = vec![(Param::Taps, taps as f64)];
            match band {
                Band::LowPass { cutoff } | Band::HighPass { cutoff } => {
                    out.push((Param::Cutoff, cutoff))
                }
                Band::BandPass { low, high } => {
                    out.push((Param::Low, low));
                    out.push((Param::High, high));
                }
            }
            out
        }
    }
}

/// `design` with one of its numbers changed. A count is a whole number, so
/// what is typed is rounded; whether the result can be realised is
/// `realize`'s to say, not this.
pub(super) fn with(design: &Design, param: Param, value: f64) -> Design {
    let count = value.round().max(0.0) as usize;
    let mut out = design.clone();
    match (&mut out, param) {
        (
            Design::MovingAverage { taps } | Design::Median { taps } | Design::Fir { taps, .. },
            Param::Taps,
        ) => *taps = count,
        (Design::Exponential { alpha }, Param::Alpha) => *alpha = value,
        (Design::Biquad { cutoff, .. } | Design::Butterworth { cutoff, .. }, Param::Cutoff) => {
            *cutoff = value
        }
        (Design::Biquad { q, .. }, Param::Q) => *q = value,
        (Design::Butterworth { order, .. }, Param::Order) => *order = count,
        (Design::Fir { band, .. }, _) => match (band, param) {
            (Band::LowPass { cutoff } | Band::HighPass { cutoff }, Param::Cutoff) => {
                *cutoff = value
            }
            (Band::BandPass { low, .. }, Param::Low) => *low = value,
            (Band::BandPass { high, .. }, Param::High) => *high = value,
            _ => {}
        },
        _ => {}
    }
    out
}

/// The rate the firmware samples at, as its own records keep it: the first
/// channel it prints on its clock, or else a converter's reports.
pub(super) fn estimated_rate(state: AppState) -> Option<f64> {
    let printed = state.sim.plot.with_untracked(|plot| {
        (plot.clock == Some(TraceClock::Firmware))
            .then(|| plot.channels.first().map(|(name, _)| name.clone()))
            .flatten()
    });
    printed
        .and_then(|name| channel_rate(state, &name))
        .or_else(|| {
            state.lab.conversions.with_untracked(|all| {
                all.values()
                    .find_map(|r| record::from_conversions(r).map(|r| r.rate))
            })
        })
}

#[component]
pub(super) fn DesignView(
    tick: RwSignal<u64>,
    played: Memo<Option<Result<Played, String>>>,
) -> impl IntoView {
    let state = AppState::expect();
    let estimate = Memo::new(move |_| {
        tick.track();
        estimated_rate(state)
    });
    let rate = move || state.lab.design_rate.get().or_else(|| estimate.get());
    let realized = move || {
        let rate = rate()?;
        Some(state.lab.design.with(|design| design.realize(rate)))
    };
    // The signal as the firmware would take it at the design's rate: a loop
    // of it, and no more than twenty seconds.
    let input = move || {
        let rate = rate()?;
        played.with(|p| {
            let played = p.as_ref()?.as_ref().ok()?;
            let seconds = (played.samples.len() as f64 / f64::from(played.rate)).clamp(1.0, 20.0);
            Some(record::from_played(played, 0, 0, rate, seconds))
        })
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col overflow-y-auto">
            <div class="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 pt-1.5">
                <select
                    on:change=move |event| {
                        let id = event_target_value(&event);
                        if let Some(kind) = Kind::ALL.into_iter().find(|k| format!("{k:?}") == id) {
                            state.lab.design.update(|design| *design = kind.design(design));
                        }
                    }
                    class="h-[22px] rounded-[5px] bg-sunken px-1 text-caption text-label-2 outline-none"
                >
                    {move || {
                        let now = state.lab.design.with(Kind::of);
                        Kind::ALL
                            .into_iter()
                            .map(|kind| view! {
                                <option value=format!("{kind:?}") selected=kind == now>{kind.label()}</option>
                            })
                            .collect_view()
                    }}
                </select>
                {move || {
                    state.lab.design.with(params).into_iter().map(|(param, value)| {
                        view! {
                            <label class="flex items-center gap-1 text-caption text-label-3">
                                <span>{param.label()}</span>
                                <input
                                    type="text"
                                    prop:value=chart::tick(value)
                                    on:change=move |event| {
                                        if let Ok(value) = event_target_value(&event).trim().parse::<f64>() {
                                            state.lab.design.update(|design| *design = with(design, param, value));
                                        }
                                    }
                                    class="h-[22px] w-[4.5rem] rounded-[5px] bg-sunken px-1.5 font-mono text-caption text-label outline-none ring-1 ring-line focus:ring-rust"
                                />
                            </label>
                        }
                    }).collect_view()
                }}
                {move || {
                    let window = state.lab.design.with(|design| match design {
                        Design::Fir { window, .. } => Some(*window),
                        _ => None,
                    })?;
                    Some(view! {
                        <select
                            title=t!("lab.window-kind-hint")
                            on:change=move |event| {
                                let chosen = match event_target_value(&event).as_str() {
                                    "blackman" => Window::Blackman,
                                    "rectangular" => Window::Rectangular,
                                    _ => Window::Hann,
                                };
                                state.lab.design.update(|design| {
                                    if let Design::Fir { window, .. } = design {
                                        *window = chosen;
                                    }
                                });
                            }
                            class="h-[22px] rounded-[5px] bg-sunken px-1 text-caption text-label-2 outline-none"
                        >
                            <option value="hann" selected=window == Window::Hann>"Hann"</option>
                            <option value="blackman" selected=window == Window::Blackman>"Blackman"</option>
                            <option value="rectangular" selected=window == Window::Rectangular>
                                {t!("lab.window-rectangular")}
                            </option>
                        </select>
                    })
                }}
                <label class="flex items-center gap-1 text-caption text-label-3" title=t!("lab.design-rate-hint")>
                    <span>{t!("lab.design-rate")}</span>
                    <input
                        type="text"
                        placeholder=move || estimate.get().map(chart::tick).unwrap_or_default()
                        prop:value=move || state.lab.design_rate.get().map(chart::tick).unwrap_or_default()
                        on:change=move |event| {
                            let text = event_target_value(&event);
                            state.lab.design_rate.set(text.trim().parse::<f64>().ok().filter(|r| *r > 0.0));
                        }
                        class="h-[22px] w-[5rem] rounded-[5px] bg-sunken px-1.5 font-mono text-caption text-label outline-none ring-1 ring-line focus:ring-rust"
                    />
                </label>
            </div>
            {move || match realized() {
                None => view! {
                    <p class="px-3 py-2 text-caption leading-relaxed text-label-3">{t!("lab.design-needs-rate")}</p>
                }
                .into_any(),
                Some(Err(refusal)) => view! {
                    <p class="px-3 py-2 text-caption leading-relaxed text-crimson">{dsp_refusal_text(&refusal)}</p>
                }
                .into_any(),
                Some(Ok(filter)) => {
                    let input = input();
                    view! { <Designed filter=filter input=input /> }.into_any()
                }
            }}
        </div>
    }
}

/// What a realised filter does: to every frequency, to the signal, and the
/// code that does the same.
#[component]
fn Designed(filter: dsp::Filter, input: Option<record::Record>) -> impl IntoView {
    const W: f64 = 1000.0;
    const H: f64 = 100.0;
    let state = AppState::expect();
    let rate = filter.rate();
    let nyquist = rate / 2.0;
    let low = (nyquist / 2000.0).max(0.01);
    // The filter's gain across the band, and the signal's spectrum behind
    // it, its peak at 0 dB, so what the filter keeps and takes out of this
    // signal is one picture.
    let freqs: Vec<f64> = (0..300)
        .map(|n| low * ((nyquist * 0.999 / low).ln() * f64::from(n) / 299.0).exp())
        .collect();
    let gain: Vec<(f64, f64)> = freqs
        .iter()
        .filter_map(|f| {
            let response = filter.response(*f)?;
            Some((
                place(*f, low, nyquist, true),
                dsp::db(response.gain).max(-100.0),
            ))
        })
        .collect();
    let spectrum = input.as_ref().map(|input| {
        let found = dsp::spectrum(&input.samples, input.rate, Window::Hann);
        let top = found
            .amplitude
            .iter()
            .skip(1)
            .copied()
            .fold(0.0, f64::max)
            .max(1e-300);
        let points: Vec<(f64, f64)> = found
            .freqs
            .iter()
            .zip(&found.amplitude)
            .filter(|(f, _)| **f >= low)
            .map(|(f, a)| (place(*f, low, nyquist, true), dsp::db(a / top).max(-100.0)))
            .collect();
        chart::path(&chart::thin(&points, 900), (-100.0, 5.0), W, H)
    });
    let response_path = chart::path(&gain, (-100.0, 5.0), W, H);
    let ticks = frequency_ticks(low, nyquist, true);

    // The signal and what the filter makes of it, from rest, as a firmware
    // that has just started would.
    let time = input.as_ref().map(|input| {
        let shown = input.samples.len().min((input.rate * 2.0) as usize).max(2);
        let before = &input.samples[..shown];
        let after = filter.apply(before);
        let range = chart::band(before.iter().chain(&after).copied()).unwrap_or((0.0, 1.0));
        let across = |values: &[f64]| {
            let last = (values.len() - 1).max(1) as f64;
            let placed: Vec<(f64, f64)> = values
                .iter()
                .enumerate()
                .map(|(n, v)| (n as f64 / last, *v))
                .collect();
            chart::path(&chart::thin(&placed, 900), range, W, H)
        };
        (across(before), across(&after), shown as f64 / input.rate)
    });

    let name = state.lab.code_name;
    let code = {
        let filter = filter.clone();
        move || filter.code(&name.get())
    };

    view! {
        <div class="flex flex-col gap-1 px-3 pb-2 pt-1">
            <div class="relative h-[110px] rounded-[4px] bg-sunken/40">
                <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" class="absolute inset-0 h-full w-full">
                    {ticks.iter().map(|f| {
                        let x = place(*f, low, nyquist, true) * W;
                        view! { <line x1=x y1="0" x2=x y2=H stroke="#3c434e" stroke-width="1" vector-effect="non-scaling-stroke" /> }
                    }).collect_view()}
                    {spectrum.map(|d| view! {
                        <path d=d fill="none" stroke="#6d7480" stroke-width="1" vector-effect="non-scaling-stroke" />
                    })}
                    <path d=response_path fill="none" stroke="#e0a838" stroke-width="1.5" vector-effect="non-scaling-stroke" />
                </svg>
                <span class="pointer-events-none absolute left-1.5 top-0.5 font-mono text-caption text-label-3">
                    {t!("lab.design-response")}
                </span>
                <span class="pointer-events-none absolute right-1.5 top-0.5 font-mono text-caption text-label-4">
                    "−100 … 5 dB"
                </span>
            </div>
            <div class="relative h-4">
                {ticks.iter().map(|f| {
                    let left = place(*f, low, nyquist, true) * 100.0;
                    view! {
                        <span class="absolute -translate-x-1/2 font-mono text-caption text-label-4" style=format!("left: {left}%")>
                            {chart::tick(*f)}
                        </span>
                    }
                }).collect_view()}
            </div>
            {time.map(|(before, after, seconds)| view! {
                <div class="relative h-[90px] rounded-[4px] bg-sunken/40">
                    <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" class="absolute inset-0 h-full w-full">
                        <path d=before fill="none" stroke=PLAYED stroke-width="1" vector-effect="non-scaling-stroke" />
                        <path d=after fill="none" stroke="#e0a838" stroke-width="1.5" vector-effect="non-scaling-stroke" />
                    </svg>
                    <span class="pointer-events-none absolute left-1.5 top-0.5 font-mono text-caption text-label-3">
                        {t!("lab.design-time", seconds = chart::tick(seconds))}
                    </span>
                </div>
            })}
            <div class="flex items-center gap-2 pt-1">
                <label class="flex items-center gap-1 text-caption text-label-3" title=t!("lab.code-name-hint")>
                    <span>{t!("lab.code-name")}</span>
                    <input
                        type="text"
                        prop:value=move || name.get()
                        on:change=move |event| name.set(event_target_value(&event).trim().to_string())
                        class="h-[22px] w-[8rem] rounded-[5px] bg-sunken px-1.5 font-mono text-caption text-label outline-none ring-1 ring-line focus:ring-rust"
                    />
                </label>
                <span class="flex-1" />
                {
                    let code = code.clone();
                    move || code().ok().map(|text| view! {
                        <button
                            type="button"
                            on:click=move |_| crate::view::components::copy_to_clipboard(&text)
                            class="rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                        >
                            {t!("lab.code-copy")}
                        </button>
                    })
                }
            </div>
            {move || match code() {
                Ok(text) => view! {
                    <pre class="max-h-[16rem] overflow-auto rounded-[6px] bg-sunken px-3 py-2 font-mono text-caption text-label-2 select-text">
                        {text}
                    </pre>
                }
                .into_any(),
                Err(refusal) => view! {
                    <p class="text-caption text-crimson">{dsp_refusal_text(&refusal)}</p>
                }
                .into_any(),
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind reads back as itself, and switching kinds keeps the
    /// corner — a notch put where the low-pass was.
    #[test]
    fn a_kind_keeps_the_corner_it_was_given() {
        let start = Design::Butterworth {
            pass: Pass::Low,
            order: 2,
            cutoff: 12.0,
        };
        for kind in Kind::ALL {
            let made = kind.design(&start);
            assert_eq!(Kind::of(&made), kind, "{made:?}");
            // A band's corner is the middle of its two edges, which is where
            // the band is put: half the corner to twice it.
            if let Some(corner) = corner_of(&made) {
                assert!((corner - 12.0).abs() < 1e-9, "{kind:?}: {corner}");
            }
        }
        // A windowed-sinc high-pass is odd, or `realize` refuses it.
        let high = Kind::FirHigh.design(&Design::MovingAverage { taps: 30 });
        assert!(matches!(high, Design::Fir { taps: 31, .. }), "{high:?}");
    }

    /// Every number a design shows can be changed, and changing one
    /// changes that one only.
    #[test]
    fn a_param_changes_what_it_names() {
        let start = Design::Fir {
            band: Band::BandPass {
                low: 5.0,
                high: 20.0,
            },
            taps: 31,
            window: Window::Hann,
        };
        let shown = params(&start);
        assert_eq!(
            shown,
            [(Param::Taps, 31.0), (Param::Low, 5.0), (Param::High, 20.0)]
        );
        let changed = with(&start, Param::High, 40.0);
        assert_eq!(
            params(&changed),
            [(Param::Taps, 31.0), (Param::Low, 5.0), (Param::High, 40.0)]
        );
        let butter = with(
            &Design::Butterworth {
                pass: Pass::High,
                order: 2,
                cutoff: 5.0,
            },
            Param::Order,
            4.4,
        );
        assert_eq!(params(&butter)[0], (Param::Order, 4.0));
    }
}
