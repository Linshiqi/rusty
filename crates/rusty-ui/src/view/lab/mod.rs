//! The signal lab (`docs/signals.md`, "The analysis"): the sheet's signals,
//! what the converter took and what the firmware printed, laid side by side
//! on the firmware's own clock — in time, as a spectrum, as a response
//! measured a tone at a time, and against a filter designed without running
//! anything.
//!
//! One tab, because the four answer one question from four sides: what does
//! this filter do to this signal. The sources column is the bench
//! generator's front panel, live while a run goes on; the rest are the
//! instruments.

use std::time::Duration;

use leptos::prelude::*;

use rusty_i18n::t;

use crate::state::{AppState, LabView};

mod chart;
mod design;
mod response;
mod sources;
mod spectrum;
mod time;

/// A signal drawn small: its path in a box `width` by `height`, and the
/// lowest and highest it reaches — the inspector's picture of what a
/// generator plays.
pub(crate) fn sparkline(samples: &[f64], width: f64, height: f64) -> (String, Option<(f64, f64)>) {
    let last = samples.len().saturating_sub(1).max(1) as f64;
    let points: Vec<(f64, f64)> = samples
        .iter()
        .enumerate()
        .map(|(n, value)| (n as f64 / last, *value))
        .collect();
    let thinned = chart::thin(&points, 160);
    let drawn = chart::band(thinned.iter().map(|(_, y)| *y)).unwrap_or((0.0, 1.0));
    let (low, high) = samples
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::MAX, f64::MIN), |(low, high), v| {
            (low.min(*v), high.max(*v))
        });
    let range = (low <= high).then_some((low, high));
    (chart::path(&thinned, drawn, width, height), range)
}

#[component]
pub fn SignalsTab() -> impl IntoView {
    let state = AppState::expect();

    // The instruments redraw on a clock of their own rather than per line:
    // a converter reports a thousand conversions a second, and a chart that
    // redrew for each would spend the run redrawing.
    let tick = RwSignal::new(0u64);
    let ticker = set_interval_with_handle(
        move || {
            if state.app.session_running.get_untracked() {
                tick.update(|n| *n += 1);
            }
        },
        Duration::from_millis(250),
    )
    .ok();
    on_cleanup(move || {
        if let Some(ticker) = ticker {
            ticker.clear();
        }
    });

    // One loop of what the studied source plays, rendered as the backend
    // renders it — again only when the source or its text changes.
    let played = Memo::new(move |_| {
        let source = state.lab.source.get()?;
        let texts = state.lab.playing.get();
        state.sim.plan.with(|plan| {
            plan.as_ref()
                .and_then(|plan| plan.board.as_ref())
                .map(|sheet| crate::lab::played(sheet, &source, &texts))
        })
    });

    view! {
        <div class="flex min-h-0 flex-1">
            <div class="flex w-[17rem] shrink-0 flex-col overflow-y-auto">
                <span class="px-3 pb-1 pt-2 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {t!("lab.sources")}
                </span>
                <sources::Sources />
                <sources::ConsoleSources />
            </div>
            <div class="w-px bg-line" />
            <div class="flex min-w-0 flex-1 flex-col">
                <Views />
                {move || {
                    let refused = played.with(|p| p.as_ref().and_then(|p| p.as_ref().err().cloned()));
                    refused.map(|why| view! {
                        <p class="px-3 pt-1 text-caption text-crimson">{why}</p>
                    })
                }}
                {move || match state.lab.view.get() {
                    LabView::Time => view! { <time::TimeView tick=tick played=played /> }.into_any(),
                    LabView::Spectrum => {
                        view! { <spectrum::SpectrumView tick=tick played=played /> }.into_any()
                    }
                    LabView::Response => {
                        view! { <response::ResponseView tick=tick played=played /> }.into_any()
                    }
                    LabView::Design => {
                        view! { <design::DesignView tick=tick played=played /> }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

/// The four instruments, one in front.
#[component]
fn Views() -> impl IntoView {
    let state = AppState::expect();
    let choices = [
        (LabView::Time, t!("lab.view-time")),
        (LabView::Spectrum, t!("lab.view-spectrum")),
        (LabView::Response, t!("lab.view-response")),
        (LabView::Design, t!("lab.view-design")),
    ];
    view! {
        <div class="flex items-center gap-1 border-b border-line px-3 py-1">
            {choices
                .into_iter()
                .map(|(view, label)| {
                    view! {
                        <button
                            type="button"
                            on:click=move |_| state.lab.view.set(view)
                            class=move || {
                                let base = "rounded-[5px] px-2 py-0.5 text-footnote";
                                if state.lab.view.get() == view {
                                    format!("{base} bg-sunken text-label")
                                } else {
                                    format!("{base} text-label-3 hover:text-label")
                                }
                            }
                        >
                            {label}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}
