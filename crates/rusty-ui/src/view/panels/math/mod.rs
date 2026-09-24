//! The math toolbox: the vectors, quaternions and frames a flight controller
//! is made of, worked out a row at a time and drawn — an attitude as the
//! aircraft it is, every step of the working in space, and the firmware's
//! own estimate beside the simulator's truth while a run goes on.
//!
//! The arithmetic and the sheet's language are `rusty_embed::spatial`,
//! pure and tested; the drawing is `crate::scene`. This is the page: the
//! rows on the left, the view over the working and the value on the right.

mod help;
mod readout;
mod rows;
mod scene;
mod steps;
mod words;

use std::time::Duration;

use leptos::prelude::*;

use rusty_embed::spatial::Frame;
use rusty_embed::spatial::sheet::{EXAMPLES, Row, evaluate};
use rusty_i18n::t;

use crate::{controller, state::AppState};

/// A row's colour in the view and beside it: eight that read apart on the
/// dark and the light theme alike, fixed for the reason the Git lanes are —
/// a row that changed colour with the theme would read as another row.
pub(super) const PALETTE: [&str; 8] = [
    "#5b9df0", "#e8a33d", "#3fb68b", "#d9658a", "#9a7bf0", "#4fc1d1", "#c7b14a", "#e06c4f",
];

pub(super) fn row_colour(index: usize) -> &'static str {
    PALETTE[index % PALETTE.len()]
}

#[component]
pub fn MathPanel() -> impl IntoView {
    let state = AppState::expect();

    // The project's sheet, whenever the project changes.
    Effect::new(move |_| {
        let _root = state
            .project
            .detected
            .with(|p| p.as_ref().map(|p| p.root.clone()));
        controller::load_math_sheet(state);
    });

    // What a running simulation says, ten times a second, for a sheet that
    // reads it — the rows re-evaluate when it changes, and not per line of
    // telemetry, which arrives hundreds of times a second.
    let ticker = set_interval_with_handle(
        move || controller::refresh_math_live(state),
        Duration::from_millis(100),
    )
    .ok();
    on_cleanup(move || {
        if let Some(ticker) = ticker {
            ticker.clear();
        }
    });

    let rows: Memo<Vec<Row>> = Memo::new(move |_| {
        let sheet = state.math.sheet.get();
        state.math.live.with(|live| evaluate(&sheet, live))
    });

    view! {
        <div class="flex h-full min-h-0 flex-col">
            <Header />
            {move || {
                state
                    .math
                    .unreadable
                    .get()
                    .map(|reason| {
                        view! {
                            <div class="flex-none border-b border-line bg-crimson-fill px-4 py-2 text-footnote text-crimson select-text">
                                {t!("math.unreadable", reason = reason)}
                            </div>
                        }
                    })
            }}
            <div class="flex min-h-0 flex-1">
                <div class="relative flex w-[360px] min-w-[280px] flex-none flex-col border-r border-line">
                    <rows::Rows rows=rows />
                    <help::Help />
                </div>
                <div class="flex min-w-0 flex-1 flex-col">
                    <div class="relative min-h-0 flex-[3]">
                        <scene::SceneView rows=rows />
                    </div>
                    // Side by side where there is room for both, one over
                    // the other where there is not: squeezed beside the
                    // value, the working was a column a word wide.
                    <div class="@container min-h-0 flex-[2] border-t border-line">
                        <div class="flex h-full min-h-0 flex-col @min-[640px]:flex-row">
                            <steps::Steps rows=rows />
                            <readout::Readout rows=rows />
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// The panel's name, the examples, which way up, and the reference.
#[component]
fn Header() -> impl IntoView {
    let state = AppState::expect();
    let frame = move || state.math.sheet.with(|s| s.frame);
    let segment = move |this: Frame| {
        move || {
            let base = "px-2 py-0.5 text-footnote rounded-[5px] transition-colors";
            if frame() == this {
                format!("{base} bg-raised text-label shadow-sm")
            } else {
                format!("{base} text-label-2 hover:text-label")
            }
        }
    };
    let saved = move || {
        if state.math.home.get().is_some() {
            t!("math.saved-in-project")
        } else if state.math.unreadable.with(Option::is_some) {
            String::new()
        } else {
            t!("math.not-saved")
        }
    };
    view! {
        <div class="flex h-[38px] flex-none items-center gap-3 border-b border-line px-4">
            <span class="text-strong font-semibold tracking-tight">{t!("panel.math")}</span>
            <span class="truncate text-footnote text-label-3">{saved}</span>
            <span class="flex-1" />
            <select
                class="h-[24px] rounded-[6px] bg-sunken px-1.5 text-footnote text-label outline-none ring-1 ring-line"
                title=t!("math.examples-hint")
                on:change=move |event| {
                    let id = event_target_value(&event);
                    if let Some(example) = EXAMPLES.iter().find(|e| e.id == id) {
                        controller::open_math_example(state, example.id);
                    }
                    if let Some(select) = event
                        .target()
                        .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlSelectElement>(t).ok())
                    {
                        select.set_selected_index(0);
                    }
                }
            >
                <option value="">{t!("math.examples")}</option>
                {EXAMPLES
                    .iter()
                    .map(|e| view! { <option value=e.id>{words::example(e.id)}</option> })
                    .collect_view()}
            </select>
            <div
                class="flex items-center gap-0.5 rounded-[7px] bg-sunken p-0.5 ring-1 ring-line"
                role="group"
            >
                <button
                    type="button"
                    class=segment(Frame::ZUp)
                    title=t!("math.frame-z-up-hint")
                    on:click=move |_| controller::set_math_frame(state, Frame::ZUp)
                >
                    {t!("math.frame-z-up")}
                </button>
                <button
                    type="button"
                    class=segment(Frame::ZDown)
                    title=t!("math.frame-z-down-hint")
                    on:click=move |_| controller::set_math_frame(state, Frame::ZDown)
                >
                    {t!("math.frame-z-down")}
                </button>
            </div>
            <button
                type="button"
                class=move || {
                    if state.math.help.get() {
                        "h-[24px] rounded-[6px] px-2 text-footnote bg-selection text-rust"
                    } else {
                        "h-[24px] rounded-[6px] px-2 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    }
                }
                title=t!("math.help-hint")
                on:click=move |_| state.math.help.update(|open| *open = !*open)
            >
                {t!("math.help")}
            </button>
        </div>
    }
}
