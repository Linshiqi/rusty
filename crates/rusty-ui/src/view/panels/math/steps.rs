//! The selected row's working, a step at a time — each the formula with its
//! numbers in, what is worth knowing about it, and, clicked, drawn in the
//! view. A row that turns something can be played: the aircraft turns
//! through its steps in order, which is what makes `euler(roll, pitch,
//! yaw)` read as the yaw, then the pitch about where the nose has got to,
//! then the roll.

use std::time::Duration;

use leptos::{ev, prelude::*};

use rusty_embed::spatial::sheet::Row;
use rusty_i18n::t;

use super::scene::turns;
use super::words;
use crate::state::AppState;

/// How long one turn takes to play.
const SECONDS_PER_TURN: f64 = 0.9;
const TICK_MS: u64 = 16;

#[component]
pub fn Steps(rows: Memo<Vec<Row>>) -> impl IntoView {
    let state = AppState::expect();
    let row = Memo::new(move |_| {
        state
            .math
            .selected
            .get()
            .and_then(|i| rows.with(|r| r.get(i).cloned()))
    });
    let turn_count = Memo::new(move |_| row.with(|r| r.as_ref().map_or(0, |r| turns(r).len())));

    let ticker = set_interval_with_handle(
        move || {
            if !state.math.playing.get_untracked() {
                return;
            }
            let n = turn_count.get_untracked();
            if n == 0 {
                state.math.playing.set(false);
                return;
            }
            let step = (TICK_MS as f64 / 1000.0) / (SECONDS_PER_TURN * n as f64);
            let next = (state.math.progress.get_untracked() + step).min(1.0);
            state.math.progress.set(next);
            if next >= 1.0 {
                state.math.playing.set(false);
            }
        },
        Duration::from_millis(TICK_MS),
    )
    .ok();
    on_cleanup(move || {
        if let Some(ticker) = ticker {
            ticker.clear();
        }
    });

    let play = move |_| {
        if state.math.playing.get_untracked() {
            state.math.playing.set(false);
            return;
        }
        if state.math.progress.get_untracked() >= 1.0 {
            state.math.progress.set(0.0);
        }
        state.math.step.set(None);
        state.math.playing.set(true);
    };

    view! {
        <div class="flex min-h-0 min-w-0 flex-1 flex-col border-b border-line @min-[640px]:border-r @min-[640px]:border-b-0">
            <div class="flex h-[32px] flex-none items-center gap-2 border-b border-line px-3">
                <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {t!("math.steps")}
                </span>
                <span class="flex-1" />
                {move || {
                    (turn_count.get() > 0)
                        .then(|| {
                            view! {
                                <button
                                    type="button"
                                    class="h-[22px] rounded-[6px] px-2 text-footnote text-rust hover:bg-sunken"
                                    on:click=play
                                >
                                    {move || {
                                        if state.math.playing.get() {
                                            t!("math.pause")
                                        } else {
                                            t!("math.play")
                                        }
                                    }}
                                </button>
                                <input
                                    type="range"
                                    class="h-[14px] w-[160px] accent-rust"
                                    min="0"
                                    max="1"
                                    step="0.001"
                                    title=t!("math.scrub-hint")
                                    prop:value=move || state.math.progress.get()
                                    on:input=move |event: ev::Event| {
                                        if let Ok(v) = event_target_value(&event).parse::<f64>() {
                                            state.math.playing.set(false);
                                            state.math.step.set(None);
                                            state.math.progress.set(v);
                                        }
                                    }
                                />
                            }
                        })
                }}
            </div>
            <div class="min-h-0 flex-1 overflow-y-auto px-3 py-2">
                {move || match row.get() {
                    None => {
                        view! { <p class="text-footnote text-label-3">{t!("math.no-row")}</p> }
                            .into_any()
                    }
                    Some(row) if row.steps.is_empty() => {
                        view! { <p class="text-footnote text-label-3">{t!("math.no-steps")}</p> }
                            .into_any()
                    }
                    Some(row) => {
                        let all_turns = turns(&row);
                        row.steps
                            .into_iter()
                            .enumerate()
                            .filter(|(_, step)| {
                                !step.lines.is_empty() || !step.remarks.is_empty()
                            })
                            .map(|(i, step)| {
                                let ends_at = all_turns
                                    .iter()
                                    .position(|(k, _)| *k == i)
                                    .map(|k| (k + 1) as f64 / all_turns.len() as f64);
                                let chosen = move || state.math.step.get() == Some(i);
                                view! {
                                    <button
                                        type="button"
                                        class=move || {
                                            let base = "mb-1.5 block w-full rounded-[7px] px-2.5 py-1.5 text-left";
                                            if chosen() {
                                                format!("{base} bg-selection ring-1 ring-rust/40")
                                            } else {
                                                format!("{base} bg-sunken/50 hover:bg-sunken")
                                            }
                                        }
                                        on:click=move |_| {
                                            state.math.playing.set(false);
                                            state.math.step.set(Some(i));
                                            if let Some(end) = ends_at {
                                                state.math.progress.set(end);
                                            }
                                        }
                                    >
                                        <div class="text-footnote font-semibold text-label">
                                            {words::what(step.what)}
                                        </div>
                                        {step
                                            .lines
                                            .into_iter()
                                            .map(|line| {
                                                view! {
                                                    <div class="font-mono text-footnote leading-relaxed text-label-2 select-text break-all">
                                                        {line}
                                                    </div>
                                                }
                                            })
                                            .collect_view()}
                                        {step
                                            .remarks
                                            .into_iter()
                                            .map(|remark| {
                                                view! {
                                                    <div class="mt-0.5 text-footnote text-label-3">
                                                        {words::remark(remark)}
                                                    </div>
                                                }
                                            })
                                            .collect_view()}
                                    </button>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}
