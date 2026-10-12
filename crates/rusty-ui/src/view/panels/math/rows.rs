//! The sheet's rows: typed, worked out as they are typed, and each saying
//! what it came to — or why it came to nothing — underneath.

use leptos::{ev, prelude::*};
use wasm_bindgen::JsCast;

use rusty_embed::spatial::sheet::steps::{deg, mat3, num, quat, vec2, vec3};
use rusty_embed::spatial::sheet::{Row, Unit, Value, set_literal};
use rusty_i18n::t;

use super::{row_colour, words};
use crate::{controller, state::AppState};

/// Is this `input` event a keystroke an input method has not finished
/// with? The wizard's rule: a row is read once `compositionend` says the
/// letters are settled, never the pinyin on the way there.
fn composing(event: &ev::Event) -> bool {
    event
        .dyn_ref::<web_sys::InputEvent>()
        .is_some_and(web_sys::InputEvent::is_composing)
}

/// Put the caret in a row's field once the row is on the page.
fn focus_row(index: usize) {
    set_timeout(
        move || {
            let Some(document) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let selector = format!("[data-math-row=\"{index}\"] input");
            if let Some(input) = document
                .query_selector(&selector)
                .ok()
                .flatten()
                .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
            {
                let _ = input.focus();
            }
        },
        std::time::Duration::ZERO,
    );
}

#[component]
pub fn Rows(rows: Memo<Vec<Row>>) -> impl IntoView {
    let state = AppState::expect();
    let count = move || state.math.sheet.with(|s| s.rows.len());
    view! {
        <div class="min-h-0 flex-1 overflow-y-auto py-1.5">
            <For each=move || 0..count() key=|index| *index let:index>
                <RowView index=index rows=rows />
            </For>
            <div class="px-3 pt-1 pb-3">
                <button
                    type="button"
                    class="flex h-[26px] w-full items-center gap-1.5 rounded-[6px] px-2 text-left text-footnote text-label-3 hover:bg-sunken hover:text-label"
                    on:click=move |_| {
                        let at = controller::add_math_row(state, None);
                        focus_row(at);
                    }
                >
                    "+ "
                    {t!("math.add-row")}
                </button>
                {move || {
                    (count() == 0)
                        .then(|| {
                            view! {
                                <p class="px-2 pt-2 text-footnote text-label-3">
                                    {t!("math.empty-rows")}
                                </p>
                            }
                        })
                }}
            </div>
        </div>
    }
}

#[component]
fn RowView(index: usize, rows: Memo<Vec<Row>>) -> impl IntoView {
    let state = AppState::expect();
    let text = move || {
        state
            .math
            .sheet
            .with(|s| s.rows.get(index).cloned().unwrap_or_default())
    };
    let worked = Memo::new(move |_| rows.with(|r| r.get(index).cloned()));
    let selected = move || state.math.selected.get() == Some(index);
    let hidden = move || {
        state
            .math
            .hidden
            .with(|h| h.get(index).copied().unwrap_or(false))
    };
    let colour = row_colour(index);

    let on_key = move |event: ev::KeyboardEvent| {
        if event.is_composing() {
            return;
        }
        match event.key().as_str() {
            "Enter" => {
                event.prevent_default();
                let at = controller::add_math_row(state, Some(index));
                focus_row(at);
            }
            "Backspace" if text().is_empty() => {
                event.prevent_default();
                controller::remove_math_row(state, index);
                focus_row(index.saturating_sub(1));
            }
            "ArrowUp" if index > 0 => {
                event.prevent_default();
                focus_row(index - 1);
            }
            "ArrowDown" => {
                let count = state.math.sheet.with_untracked(|s| s.rows.len());
                if index + 1 < count {
                    event.prevent_default();
                    focus_row(index + 1);
                }
            }
            _ => {}
        }
    };

    view! {
        <div
            data-math-row=index
            class=move || {
                let base = "group relative mx-1.5 rounded-[7px] px-2 py-1";
                if selected() {
                    format!("{base} bg-selection")
                } else {
                    format!("{base} hover:bg-sunken/60")
                }
            }
            on:mousedown=move |_| controller::select_math_row(state, Some(index))
        >
            <div class="flex items-center gap-1.5">
                <button
                    type="button"
                    class="flex h-[18px] w-[18px] flex-none items-center justify-center rounded-full"
                    title=move || {
                        if hidden() { t!("math.show-row") } else { t!("math.hide-row") }
                    }
                    on:click=move |event| {
                        event.stop_propagation();
                        controller::toggle_math_row(state, index);
                    }
                >
                    <span
                        class="block h-[9px] w-[9px] rounded-full"
                        style=move || {
                            if hidden() {
                                format!("border: 1.5px solid {colour}; background: transparent")
                            } else {
                                format!("background: {colour}")
                            }
                        }
                    />
                </button>
                <input
                    class="h-[24px] min-w-0 flex-1 rounded-[5px] bg-transparent px-1 font-mono text-callout text-label outline-none focus:bg-content focus:ring-1 focus:ring-line"
                    spellcheck="false"
                    placeholder=t!("math.row-placeholder")
                    prop:value=text
                    on:focus=move |_| controller::select_math_row(state, Some(index))
                    on:input=move |event: ev::Event| {
                        if composing(&event) {
                            return;
                        }
                        controller::set_math_row(state, index, event_target_value(&event));
                    }
                    on:compositionend=move |event: ev::CompositionEvent| {
                        controller::set_math_row(state, index, event_target_value(&event));
                    }
                    on:keydown=on_key
                />
                <button
                    type="button"
                    class="flex h-[20px] w-[20px] flex-none items-center justify-center rounded-[5px] text-label-3 opacity-0 group-hover:opacity-100 hover:bg-sunken hover:text-crimson"
                    title=t!("math.remove-row")
                    on:click=move |event| {
                        event.stop_propagation();
                        controller::remove_math_row(state, index);
                    }
                >
                    "×"
                </button>
            </div>
            {move || worked.get().map(|row| view! { <Outcome row=row index=index /> })}
        </div>
    }
}

/// What a row came to, under it: the value, or why there is none, the
/// notes worth reading, and a slider for a row that is one number.
#[component]
fn Outcome(row: Row, index: usize) -> impl IntoView {
    let state = AppState::expect();
    let live = row.live.then(|| {
        view! {
            <span class="rounded-[4px] bg-patina-fill px-1 text-caption font-semibold tracking-[0.04em] text-patina uppercase">
                {t!("math.live")}
            </span>
        }
    });
    let value = match &row.value {
        None => None,
        Some(Ok(value)) => Some(
            view! {
                <div class="flex min-w-0 items-center gap-1.5 pl-[26px] font-mono text-footnote text-label-2 select-text">
                    <span class="truncate">{show(value)}</span>
                    {live}
                </div>
            }
            .into_any(),
        ),
        Some(Err(problem)) => Some(
            view! {
                <div class="pl-[26px] text-footnote text-crimson select-text">
                    {words::problem(problem)}
                </div>
            }
            .into_any(),
        ),
    };
    let notes = row
        .notes
        .iter()
        .map(|note| {
            view! { <div class="pl-[26px] text-footnote text-amber">{words::note(note)}</div> }
        })
        .collect_view();
    let slider = row.slider.map(|slider| {
        let shown = match slider.unit {
            Unit::Degrees => format!("{}°", trimmed(slider.value, 1)),
            Unit::Radians => format!("{} rad", trimmed(slider.value, 3)),
            Unit::Plain => trimmed(slider.value, 2),
        };
        view! {
            <div class="flex items-center gap-2 pt-0.5 pl-[26px]">
                <input
                    type="range"
                    class="h-[14px] min-w-0 flex-1 accent-rust"
                    min=slider.min
                    max=slider.max
                    step=slider.step
                    prop:value=slider.value
                    on:mousedown=|event| event.stop_propagation()
                    on:input=move |event: ev::Event| {
                        let Ok(value) = event_target_value(&event).parse::<f64>() else {
                            return;
                        };
                        let text = state
                            .math
                            .sheet
                            .with_untracked(|s| s.rows.get(index).cloned().unwrap_or_default());
                        if let Some(next) = set_literal(&text, value) {
                            controller::set_math_row(state, index, next);
                        }
                    }
                />
                <span class="w-[64px] flex-none text-right font-mono text-footnote text-label-2 tnum">
                    {shown}
                </span>
            </div>
        }
    });
    view! {
        {value}
        {notes}
        {slider}
    }
}

fn trimmed(v: f64, places: usize) -> String {
    let text = format!("{v:.places$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// A value on one line, as the row shows it.
pub(super) fn show(value: &Value) -> String {
    match value {
        Value::Number(v) => num(*v),
        Value::Angle(v) => format!("{}  ({} rad)", deg(*v), num(*v)),
        Value::Vec2(v) => vec2(*v),
        Value::Vec3(v) => vec3(*v),
        Value::Quat(q) => quat(*q),
        Value::Euler(e) => format!(
            "{} {} · {} {} · {} {}",
            t!("math.readout.roll"),
            deg(e.roll),
            t!("math.readout.pitch"),
            deg(e.pitch),
            t!("math.readout.yaw"),
            deg(e.yaw)
        ),
        Value::Mat3(m) => mat3(*m),
        Value::Verdict(v) => {
            let error = if v.angle { deg(v.error) } else { num(v.error) };
            format!("{} · Δ {error}", words::relation(v.relation))
        }
    }
}
