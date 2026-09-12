//! The chip's pins, from the status bar.
//!
//! Answers "where did I put the LED, and what is still free" without leaving
//! the file — the question that otherwise means grepping for `GPIO` and then
//! checking a datasheet for whether the answer is allowed.
//!
//! **A chip diagram, not a devkit.** The pins are drawn in numeric order in
//! two columns, because what rusty knows is which pins the *part* has; where
//! they come out on a particular module's header is a property of the board
//! and is not guessed at here. Every square on screen is a pin that exists.
//!
//! It sits at the right end of the status bar — where a dependency count and
//! a board count used to sit, two numbers nobody acted on — and opens
//! upwards on click, as the chip's popover at the other end does. It used
//! to float over the editor's bottom-right corner and stay there, open by
//! default and remembered across launches, which read as a fixture in the
//! way of the text. Closed by default and not remembered now: it is one
//! click away, and a click on a pin closes it, since the jump is the answer.
//!
//! Read-only on purpose, for now. Editing a pin from here means writing into
//! a buffer the editor owns — its undo history, its language server — and
//! that is a correctness problem worth its own pass rather than a corner of
//! this one.

use leptos::{ev, prelude::*};

use rusty_embed::{PinInfo, PinReport};

use rusty_i18n::t;

use crate::{controller, state::AppState};

/// The status bar's pin item: the chip's name, and the pin map above it on
/// click. Nothing while no chip has a pin report — an item for a project
/// with no chip would name nothing.
#[component]
pub fn PinStatus() -> impl IntoView {
    let state = AppState::expect();
    let open = RwSignal::new(false);
    // The corner's collapsed state, from before this was a status-bar item.
    // Read once and dropped, so the storage audit stays a grep.
    let _ = crate::state::local_take("rusty.pinmap.open");

    // Re-read when the project changes: a chip switch changes every answer
    // on this panel.
    Effect::new(move |_| {
        if state.has_project() {
            controller::load_pin_report(state);
        }
    });

    move || {
        let report = state.project.pins.get()?;
        let chip = report.chip.to_uppercase();
        let label = t!("pinmap.pins", chip = chip);
        Some(view! {
            <div class="relative h-full">
                <button
                    type="button"
                    title=move || if open.get() { t!("pinmap.hide") } else { t!("pinmap.show") }
                    on:click=move |_| open.update(|it| *it = !*it)
                    class=move || {
                        format!(
                            "flex h-full items-center gap-1.5 border-l border-line px-3 transition-colors \
                             hover:bg-sunken hover:text-label {}",
                            if open.get() { "bg-sunken text-label" } else { "" },
                        )
                    }
                >
                    {label}
                    <span class="text-label-4">"▴"</span>
                </button>
                {move || {
                    open.get()
                        .then(|| {
                            let report = report.clone();
                            view! {
                                // Full-screen catcher, so clicking anywhere
                                // else closes it — the behaviour every menu
                                // in here has.
                                <div class="fixed inset-0 z-40" on:click=move |_| open.set(false) />
                                <div
                                    class="absolute right-0 bottom-full z-50 mb-px flex max-h-[70vh] w-[16rem] flex-col rounded-t-[8px] border border-line bg-raised pt-1.5 shadow-lg"
                                    // A pin is a jump to where it is named; the
                                    // jump is the answer, so it closes the map.
                                    on:click=move |event: ev::MouseEvent| {
                                        use wasm_bindgen::JsCast;
                                        let on_button = event
                                            .target()
                                            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                                            .and_then(|element| element.closest("button").ok().flatten())
                                            .is_some();
                                        if on_button {
                                            open.set(false);
                                        }
                                    }
                                >
                                    <Body report=report />
                                </div>
                            }
                        })
                }}
            </div>
        })
    }
}

#[component]
fn Body(report: PinReport) -> impl IntoView {
    let state = AppState::expect();
    let claimed = report
        .pins
        .iter()
        .filter(|pin| !pin.claims.is_empty())
        .count();
    // The two columns are the diagram: numeric order, split in half, which is
    // the only arrangement every part shares.
    let half = report.pins.len().div_ceil(2);
    let (left, right) = report.pins.split_at(half.min(report.pins.len()));
    let (left, right) = (left.to_vec(), right.to_vec());

    // A pin the source names that the part does not have is the whole of the
    // work after a chip switch, so it leads rather than hiding in a column.
    let unknown = report.unknown.clone();
    let note = report.note.clone();
    // With no pin table read, a claim the table does not list is not "not on
    // this part" — nothing here knows what is on the part. It is unverified,
    // and reads as such: a neutral list, not a red one.
    let blind = report.pins.is_empty();
    let (unknown_box, unknown_row) = if blind {
        (
            "mb-1.5 rounded-[6px] bg-sunken px-2 py-1.5",
            "block w-full text-left font-mono text-caption text-label-3 hover:underline",
        )
    } else {
        (
            "mb-1.5 rounded-[6px] bg-crimson-fill px-2 py-1.5",
            "block w-full text-left font-mono text-caption text-crimson hover:underline",
        )
    };

    view! {
        <div class="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
            {note
                .map(|text| {
                    view! {
                        <p class="mb-1.5 rounded-[6px] bg-sunken px-2 py-1.5 text-caption leading-relaxed text-label-3 select-text">
                            {text}
                        </p>
                    }
                })}
            {(!unknown.is_empty())
                .then(|| {
                    view! {
                        <div class=unknown_box>
                            {unknown
                                .into_iter()
                                .map(|claim| {
                                    let (file, line) = (claim.file.clone(), claim.line);
                                    let label = if blind {
                                        t!("pinmap.unverified", gpio = claim.gpio.to_string())
                                    } else {
                                        t!("pinmap.not-on-part", gpio = claim.gpio.to_string())
                                    };
                                    view! {
                                        <button
                                            type="button"
                                            on:click=move |_| {
                                                controller::open_at(state.focused(), file.clone(), line, 0)
                                            }
                                            class=unknown_row
                                        >
                                            {label}
                                        </button>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                })}
            <div class="flex gap-1">
                <Column pins=left />
                <Column pins=right />
            </div>
            {(!report.pins.is_empty())
                .then(|| {
                    view! {
                        <p class="mt-1.5 text-caption leading-relaxed text-label-4">
                            {t!(
                                "pinmap.claimed",
                                claimed = claimed.to_string(),
                                total = report.pins.len().to_string()
                            )}
                        </p>
                    }
                })}
        </div>
    }
}

#[component]
fn Column(pins: Vec<PinInfo>) -> impl IntoView {
    let state = AppState::expect();
    view! {
        <div class="flex min-w-0 flex-1 flex-col gap-px">
            {pins
                .into_iter()
                .map(|pin| {
                    let used = pin.claims.first().cloned();
                    let reserved = pin.reserved.clone();
                    // Reserved *and* claimed is the one combination worth
                    // interrupting for: it compiles, and the board stops
                    // booting. Everything else is information.
                    let tone = match (&reserved, &used) {
                        (Some(_), Some(_)) => "bg-crimson-fill text-crimson",
                        (Some(_), None) => "text-label-4",
                        (None, Some(_)) => "bg-selection text-rust",
                        (None, None) => "text-label-3",
                    };
                    let mut hint = format!("GPIO{}", pin.gpio);
                    if pin.input_only {
                        hint.push_str(&format!(" · {}", t!("pinmap.input-only")));
                    }
                    if let Some(reserved) = &reserved {
                        hint.push_str(&format!(" · {reserved}"));
                    }
                    if !pin.analog.is_empty() {
                        hint.push_str(&format!(" · {}", pin.analog.join(", ")));
                    }
                    match &used {
                        Some(claim) => {
                            hint.push_str(&format!("\n{}:{}\n{}", claim.file, claim.line + 1, claim.text));
                        }
                        None => hint.push_str(&format!("\n{}", t!("pinmap.free"))),
                    }
                    let jump = used.clone();
                    view! {
                        <button
                            type="button"
                            title=hint
                            disabled=jump.is_none()
                            on:click=move |_| {
                                if let Some(claim) = &jump {
                                    controller::open_at(state.focused(), claim.file.clone(), claim.line, 0);
                                }
                            }
                            class=format!(
                                "flex items-baseline gap-1 rounded-[3px] px-1 py-px text-left font-mono text-caption transition-colors disabled:pointer-events-none {tone}",
                            )
                        >
                            <span class="w-[3.5ch] shrink-0">{pin.gpio}</span>
                            <span class="min-w-0 truncate opacity-80">
                                {reserved
                                    .map(|r| r.split(" (").next().unwrap_or(&r).to_string())
                                    .or_else(|| {
                                        used.as_ref().map(|c| {
                                            c.file.rsplit('/').next().unwrap_or(&c.file).to_string()
                                        })
                                    })
                                    .unwrap_or_default()}
                            </span>
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}
