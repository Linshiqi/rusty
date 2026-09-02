//! "Set up your environment" — the screen a fresh machine gets.
//!
//! Everything it needs already existed: the probe, the recipes, the archive
//! downloads. What did not exist was anyone running them without being asked.
//! Somebody who had just installed the app had to find the Toolchain panel
//! and press the right buttons in the right order, and nothing said which
//! order that was.
//!
//! Three things this screen must do and one it must not.
//!
//! - **Say what is missing and why.** Each row carries the tool's own purpose
//!   from the catalogue, because "install six things" with no reasons is a
//!   dialog people click through and then cannot debug.
//! - **Say where it will land, before it runs.** Three different homes are
//!   involved and only one of them is rusty's to choose.
//! - **Do it in one gesture**, in an order that works, stopping at the first
//!   failure rather than reporting a ready machine that is not.
//!
//! And it must not appear when there is nothing to do. A first-run check that
//! shows up on a working machine is one people dismiss without reading, and
//! then dismiss the time it mattered.

use leptos::prelude::*;

use rusty_embed::setup::Destination;

use crate::{
    controller,
    state::AppState,
    view::components::{Dot, Tone},
    view::icon::{Icon, IconView},
};

#[component]
pub fn SetupSheet() -> impl IntoView {
    let state = AppState::expect();

    move || {
        if !state.setup.open.get() {
            return ().into_any();
        }
        let steps = state.setup.steps.get();
        let running = state.setup.running.get();
        let installed = state.setup.installed.get();
        let failed = state.setup.failed.get();
        let busy = running.is_some();
        // Where downloaded tools land. The only path of the three that rusty
        // owns, so the only one there is any point offering to change.
        let data_dir = state
            .setup
            .data_dir
            .get()
            .unwrap_or_else(|| "rusty's data directory".to_string());

        let ready = steps.is_empty();

        view! {
            <div class="absolute inset-0 z-40 flex items-center justify-center bg-canvas/80 p-8">
                <div class="flex max-h-full w-[640px] flex-col overflow-hidden rounded-[10px] border border-line bg-content shadow-2xl">
                    <div class="flex flex-none items-start gap-3 border-b border-line px-5 py-4">
                        <div class="mt-0.5 text-rust">
                            <IconView icon=Icon::Toolchain size=20 />
                        </div>
                        <div class="min-w-0 flex-1">
                            <h2 class="text-body font-medium">
                                {if ready {
                                    "This machine is ready"
                                } else {
                                    "A few things are missing"
                                }}
                            </h2>
                            <p class="mt-0.5 text-callout leading-relaxed text-label-2">
                                {if ready {
                                    "Everything rusty needs to build, flash and simulate is \
                                     installed."
                                        .to_string()
                                } else {
                                    format!(
                                        "rusty can install {} of these for you, in this order. \
                                         Every command runs in the dock where you can read it.",
                                        steps.iter().filter(|s| s.manual.is_none()).count(),
                                    )
                                }}
                            </p>
                        </div>
                    </div>

                    <div class="min-h-0 flex-1 overflow-y-auto">
                        {steps
                            .iter()
                            .enumerate()
                            .map(|(index, step)| {
                                let done = installed.contains(&step.tool);
                                let bad = failed.contains(&step.tool);
                                let now = running == Some(index);
                                let tone = if bad {
                                    Tone::Crimson
                                } else if done {
                                    Tone::Patina
                                } else if now {
                                    Tone::Amber
                                } else {
                                    Tone::Neutral
                                };
                                let where_to = controller::destination_label(step, &data_dir);
                                let manual = step.manual.clone();
                                let command = step.command.clone();
                                let name = step.tool.clone();
                                let purpose = step.purpose.clone();
                                let slow = step.slow;
                                view! {
                                    <div class="flex gap-2.5 border-b border-line px-5 py-3 last:border-b-0">
                                        <div class="mt-[5px]">
                                            <Dot tone=tone />
                                        </div>
                                        <div class="min-w-0 flex-1">
                                            <div class="flex items-baseline gap-2">
                                                <span class="font-mono text-footnote text-label">
                                                    {name}
                                                </span>
                                                {slow
                                                    .then(|| {
                                                        view! {
                                                            <span class="text-caption text-label-3">
                                                                "takes several minutes"
                                                            </span>
                                                        }
                                                    })}
                                                {now
                                                    .then(|| {
                                                        view! {
                                                            <span class="text-caption text-amber">
                                                                "installing…"
                                                            </span>
                                                        }
                                                    })}
                                            </div>
                                            <p class="mt-0.5 text-callout leading-relaxed text-label-2">
                                                {manual.clone().unwrap_or(purpose)}
                                            </p>
                                            // The command and its destination,
                                            // before it runs. A one-click
                                            // installer that does not say what
                                            // it is about to run on your
                                            // machine is one nobody should
                                            // click.
                                            {(!command.is_empty())
                                                .then(|| {
                                                    view! {
                                                        <p class="mt-1 font-mono text-caption text-label-3">
                                                            {command}
                                                        </p>
                                                        <p class="text-caption text-label-4">
                                                            "→ " {where_to}
                                                        </p>
                                                    }
                                                })}
                                            {manual
                                                .is_some()
                                                .then(|| {
                                                    view! {
                                                        <button
                                                            type="button"
                                                            on:click=move |_| {
                                                                controller::open_url(
                                                                    state,
                                                                    "https://rustup.rs".to_string(),
                                                                )
                                                            }
                                                            class="mt-1.5 rounded-[6px] border border-line px-2.5 py-1 text-caption text-label-2 hover:bg-sunken hover:text-label"
                                                        >
                                                            "Open rustup.rs"
                                                        </button>
                                                    }
                                                })}
                                        </div>
                                    </div>
                                }
                            })
                            .collect_view()}
                    </div>

                    <div class="flex flex-none items-center gap-2 border-t border-line px-5 py-3">
                        // Only the downloads have a choosable home, so that is
                        // the only one offered. A picker that claimed to move
                        // `~/.cargo/bin` would be lying about somebody's disk.
                        {(!ready)
                            .then(|| {
                                let has_downloads = steps
                                    .iter()
                                    .any(|s| s.destination == Destination::DataDirectory);
                                has_downloads
                                    .then(|| {
                                        view! {
                                            <button
                                                type="button"
                                                title="Downloaded emulators, debuggers and compilers live here"
                                                disabled=busy
                                                on:click=move |_| {
                                                    // Settings owns relocation,
                                                    // including the awkward
                                                    // case this modal would
                                                    // have to reinvent: a
                                                    // target directory that
                                                    // already has data in it.
                                                    let crate::view::SettingsOpen(open) =
                                                        expect_context::<crate::view::SettingsOpen>();
                                                    controller::close_setup(state);
                                                    open.set(true);
                                                }
                                                class="rounded-[6px] border border-line px-2.5 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-40"
                                            >
                                                "Change where downloads go…"
                                            </button>
                                        }
                                    })
                            })}
                        <span class="flex-1" />
                        <button
                            type="button"
                            disabled=busy
                            on:click=move |_| controller::close_setup(state)
                            class="rounded-[6px] px-3 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-40"
                        >
                            {if ready { "Done" } else { "Not now" }}
                        </button>
                        {(!ready)
                            .then(|| {
                                let installable = steps.iter().any(|s| s.manual.is_none());
                                installable
                                    .then(|| {
                                        view! {
                                            <button
                                                type="button"
                                                disabled=busy
                                                on:click=move |_| controller::install_all(state)
                                                class="rounded-[6px] bg-rust px-3 py-1.5 text-footnote font-medium text-canvas hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                                            >
                                                {if busy { "Installing…" } else { "Install everything" }}
                                            </button>
                                        }
                                    })
                            })}
                    </div>
                </div>
            </div>
        }
        .into_any()
    }
}
