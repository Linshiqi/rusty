//! The bottom dock.
//!
//! Every serious development tool has one, for a reason that only shows up in
//! use: a flash fails while you are reading the memory report, and you need to
//! see both. Panels that each own their output force you to leave the thing you
//! were doing to find out what went wrong.
//!
//! So output is shared state, not panel state, and this is the one place that
//! renders it.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    state::{AppState, Divider, DockTab},
    view::components::Tone,
    view::icon::{Icon, IconView},
};

#[component]
pub fn Dock() -> impl IntoView {
    let state = AppState::expect();

    view! {
        // The handle sits above the tab strip so the whole dock resizes, not
        // just its contents.
        <Show when=move || state.layout.dock_open.get()>
            <crate::view::split::Handle divider=Divider::Dock />
        </Show>
        <section class="flex flex-none flex-col bg-window">
            <DockTabs />
            <Show when=move || state.layout.dock_open.get()>
                // A column rather than one scrolling box: the terminal keeps its
                // prompt pinned below the scrollback, which only works if the
                // tab owns its own scroll region.
                <div
                    class="flex min-h-0 flex-col border-t border-line bg-content"
                    style=move || format!("height: {}px", state.layout.dock_height.get())
                >
                    {move || match state.layout.dock_tab.get() {
                        DockTab::Problems => view! { <ProblemsTab /> }.into_any(),
                        DockTab::Output => view! { <OutputTab /> }.into_any(),
                        DockTab::Terminal => {
                            view! { <crate::view::terminal::TerminalView /> }.into_any()
                        }
                        DockTab::Waves => {
                            view! { <crate::view::waves::WavesTab /> }.into_any()
                        }
                        DockTab::Plot => {
                            view! { <crate::view::plot::Plot /> }.into_any()
                        }
                        DockTab::Debug => view! { <DebugTab /> }.into_any(),
                        DockTab::Registers => view! { <RegistersTab /> }.into_any(),
                        DockTab::Devices => view! { <DevicesTab /> }.into_any(),
                        DockTab::Flight => view! { <FlightTab /> }.into_any(),
                    }}
                </div>
            </Show>
        </section>
    }
}

#[component]
fn DockTabs() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <div class="flex h-8 items-center gap-0.5 border-t border-line px-2">
            {DockTab::ALL
                .into_iter()
                .map(|tab| {
                    let selected = Signal::derive(move || {
                        state.layout.dock_open.get() && state.layout.dock_tab.get() == tab
                    });
                    view! {
                        <button
                            type="button"
                            on:click=move |_| {
                                // Clicking the tab you are already on collapses
                                // the dock. That is how every editor behaves,
                                // and it saves reaching for a separate control.
                                if selected.get() {
                                    state.layout.dock_open.set(false);
                                } else {
                                    state.show_dock(tab);
                                }
                            }
                            class=move || {
                                let base = "flex h-[26px] items-center gap-1.5 rounded-[5px] px-2.5 \
                                            text-callout transition-colors";
                                if selected.get() {
                                    format!("{base} bg-sunken font-medium text-label")
                                } else {
                                    format!("{base} text-label-2 hover:text-label")
                                }
                            }
                        >
                            {tab.label()}
                            <DockCount tab=tab />
                        </button>
                        // The shell picker sits beside its tab, not at the
                        // far edge next to the collapse chevron — two
                        // unrelated dropdown arrows in one corner read as
                        // one broken control.
                        {(tab == DockTab::Terminal)
                            .then(|| {
                                view! {
                                    <Show when=move || selected.get()>
                                        <ShellPicker />
                                    </Show>
                                }
                            })}
                    }
                })
                .collect_view()}

            <span class="flex-1" />

            <Show when=move || state.layout.dock_tab.get() == DockTab::Output && state.layout.dock_open.get()>
                // VSCode's pair: which channel, then a text filter. The
                // channel list is fixed — it is the set of things rusty runs.
                <select
                    title=t!("dock.chrome.channel")
                    class="h-6 rounded-[5px] bg-sunken px-1 text-footnote text-label-2 outline-none"
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        let pick = CHANNELS
                            .iter()
                            .find(|c| **c == value)
                            .copied()
                            .unwrap_or("all");
                        state.dock.pick.set(pick);
                    }
                >
                    {CHANNELS
                        .iter()
                        .map(|c| {
                            let channel = *c;
                            view! {
                                <option
                                    value=channel
                                    selected=move || state.dock.pick.get() == channel
                                >
                                    {channel}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
                // No Follow or Clear buttons: following is the default (a
                // scroll up detaches, the bottom reattaches), and Clear
                // lives in the right-click menu with the other verbs.
                <input
                    placeholder=t!("dock.chrome.filter")
                    title=t!("dock.chrome.filter-hint")
                    class="h-6 w-40 rounded-[5px] bg-sunken px-1.5 text-footnote outline-none placeholder:text-label-4"
                    prop:value=move || state.dock.filter.get()
                    on:input=move |event| state.dock.filter.set(event_target_value(&event))
                />
            </Show>

            <button
                type="button"
                title=move || {
                    if state.layout.dock_open.get() {
                        t!("dock.chrome.collapse")
                    } else {
                        t!("dock.chrome.expand")
                    }
                }
                class="grid size-6 place-items-center rounded-[5px] text-label-2 hover:bg-sunken hover:text-label"
                on:click=move |_| state.layout.dock_open.update(|open| *open = !*open)
            >
                <span class=move || {
                    if state.layout.dock_open.get() {
                        "grid transition-transform"
                    } else {
                        "grid rotate-180 transition-transform"
                    }
                }>
                    <IconView icon=Icon::Chevron size=13 />
                </span>
            </button>
        </div>
    }
}

/// The count beside a tab name. Absent rather than zero — a badge showing "0"
/// is a badge drawing attention to nothing.
#[component]
fn DockCount(tab: DockTab) -> impl IntoView {
    let state = AppState::expect();

    move || {
        let (count, tone) = match tab {
            DockTab::Problems => {
                let blocking = state.blocking_count();
                let (errors, warnings) = state.diag_counts();
                let total = state.problems().len() + errors + warnings;
                (
                    total,
                    if blocking > 0 || errors > 0 {
                        Tone::Crimson
                    } else {
                        Tone::Amber
                    },
                )
            }
            DockTab::Output => (state.dock.lines.with(Vec::len), Tone::Neutral),
            DockTab::Terminal => (0, Tone::Neutral),
            DockTab::Waves => (0, Tone::Neutral),
            // How many channels the firmware is currently talking about. The
            // one number worth glancing at from another tab: it says whether
            // the telemetry is arriving at all.
            DockTab::Plot => (state.sim.plot.with(|p| p.channels.len()), Tone::Neutral),
            DockTab::Devices => (0, Tone::Neutral),
            // How many motors are being driven right now. Worth a glance
            // from another tab for one reason: it is not zero when it should
            // be zero.
            DockTab::Flight => (
                state
                    .sim
                    .pwm
                    .with(|p| p.values().filter(|d| **d > 0.01).count()),
                Tone::Rust,
            ),
            // The frame count while stopped: a badge that says how deep
            // the target is, without opening the tab.
            DockTab::Registers => (0, Tone::Neutral),
            DockTab::Debug => (
                state.debug.session.with(|d| {
                    d.as_ref()
                        .filter(|d| !d.running)
                        .map_or(0, |d| d.stack.len())
                }),
                Tone::Rust,
            ),
        };

        (count > 0).then(|| {
            let text = match tone {
                Tone::Crimson => "text-crimson",
                Tone::Amber => "text-amber",
                _ => "text-label-3",
            };
            view! {
                <span class=format!("tnum font-mono text-footnote {text}")>{count.to_string()}</span>
            }
        })
    }
}

/// Every channel a line can carry. "all" is the view's word, not a tag.
const CHANNELS: [&str; 9] = [
    "all", "build", "flash", "monitor", "link", "simulate", "commands", "tools", "app",
];

/// True when the line passes the Output panel's filter box. Terms AND
/// together; a `!` prefix excludes. Case-insensitive, as every log filter
/// people actually use is.
fn passes_filter(text: &str, filter: &str) -> bool {
    let haystack = text.to_lowercase();
    filter.split_whitespace().all(|term| {
        if let Some(excluded) = term.strip_prefix('!') {
            excluded.is_empty() || !haystack.contains(&excluded.to_lowercase())
        } else {
            haystack.contains(&term.to_lowercase())
        }
    })
}

/// Where a right-click on a diagnostic row landed, and what the row named.
#[derive(Clone)]
struct DiagMenuAt {
    x: f64,
    y: f64,
    path: String,
    line: u32,
    col: u32,
    message: String,
}

mod debug;
mod devices;
mod flight;
mod output;
mod problems;
mod registers;
mod shell;

use debug::*;
use devices::*;
use flight::*;
use output::*;
use problems::*;
use registers::*;
use shell::*;
