//! The chip's pins, over the editor's bottom-right corner.
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
//! Read-only on purpose, for now. Editing a pin from here means writing into
//! a buffer the editor owns — its undo history, its language server — and
//! that is a correctness problem worth its own pass rather than a corner of
//! this one.

use leptos::prelude::*;

use rusty_embed::{PinInfo, PinReport};

use rusty_i18n::t;

use crate::{controller, state::AppState};

/// Collapsed state lives in localStorage: only this window cares, and losing
/// it costs a click.
const KEY: &str = "rusty.pinmap.open";

fn stored_open() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(KEY).ok().flatten())
        .is_none_or(|value| value != "0")
}

fn remember(open: bool) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(KEY, if open { "1" } else { "0" });
    }
}

#[component]
pub fn PinMap() -> impl IntoView {
    let state = AppState::expect();
    let open = RwSignal::new(stored_open());

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
        Some(if open.get() {
            view! {
                <div class="pointer-events-auto absolute right-3 bottom-3 z-20 flex max-h-[60%] w-[15rem] flex-col rounded-[8px] border border-line bg-raised/95 shadow-lg backdrop-blur">
                    <button
                        type="button"
                        title=t!("pinmap.hide")
                        on:click=move |_| {
                            open.set(false);
                            remember(false);
                        }
                        class="flex items-center justify-between px-2.5 py-1.5 text-caption text-label-3 transition-colors hover:text-label"
                    >
                        <span class="font-mono">{chip}</span>
                        <span>"▾"</span>
                    </button>
                    <Body report=report />
                </div>
            }
            .into_any()
        } else {
            view! {
                <button
                    type="button"
                    title=t!("pinmap.show")
                    on:click=move |_| {
                        open.set(true);
                        remember(true);
                    }
                    class="pointer-events-auto absolute right-3 bottom-3 z-20 rounded-[8px] border border-line bg-raised/95 px-2.5 py-1.5 font-mono text-caption text-label-3 shadow-lg transition-colors hover:text-label"
                >
                    {chip}" pins"
                </button>
            }
            .into_any()
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
                        <div class="mb-1.5 rounded-[6px] bg-crimson-fill px-2 py-1.5">
                            {unknown
                                .into_iter()
                                .map(|claim| {
                                    let (file, line) = (claim.file.clone(), claim.line);
                                    view! {
                                        <button
                                            type="button"
                                            on:click=move |_| {
                                                controller::open_at(state, file.clone(), line, 0)
                                            }
                                            class="block w-full text-left font-mono text-caption text-crimson hover:underline"
                                        >
                                            "GPIO"{claim.gpio}" — not on this part"
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
                            {claimed}" of "{report.pins.len()}" named in your source. \
                             A pin reached through a binding is not seen here."
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
                        hint.push_str(" · input-only");
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
                        None => hint.push_str("\nfree"),
                    }
                    let jump = used.clone();
                    view! {
                        <button
                            type="button"
                            title=hint
                            disabled=jump.is_none()
                            on:click=move |_| {
                                if let Some(claim) = &jump {
                                    controller::open_at(state, claim.file.clone(), claim.line, 0);
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
