//! The project's verbs, in the title bar — and the file finder's icon, which
//! is the other thing a title bar's centre holds here.
//!
//! Build, Run, Debug and Flash used to sit in the left rail, registered by
//! whichever panel was on screen. So Run was in one place on the Files panel,
//! in another on Simulate, and nowhere at all on Git; and the debugger's five
//! transport buttons pushed it down the column the moment a session began.
//! They stand beside the project's name now, where Xcode and CLion put them:
//! the title bar already says what is open, and this is what you do with it.
//! One position on every panel, in a row the window was already spending.
//!
//! Run and Debug still switch to the Simulate panel, so the board is on
//! screen while the build streams to the dock. The objection to a top row —
//! that it put Run far from the panel it switches to — was answered by the
//! button doing the switching itself.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, DockTab},
    view::icon::{Icon, IconView},
};

const BUTTON: &str = "grid size-7 place-items-center rounded-[6px] transition-colors \
                      hover:bg-sunken disabled:pointer-events-none disabled:opacity-40";

#[component]
pub fn RunControls() -> impl IntoView {
    let state = AppState::expect();
    let running = state.app.session_running;

    // Why Run cannot start, or `None` when it can. The plan is the one
    // derivation of that — the Simulate panel's tools card reads the same
    // fields — so the two cannot disagree about whether the machine is ready.
    let run_block = Signal::derive(move || {
        state.sim.plan.with(|plan| match plan {
            None => Some(t!("simulate.planning")),
            Some(plan) if !plan.supported => Some(plan.reason.clone().unwrap_or_default()),
            Some(plan) if !plan.missing.is_empty() => Some(t!("toolbar.run-blocked")),
            Some(_) => None,
        })
    });
    // Debug needs the chip's gdb on top of everything Run needs.
    let debug_block = Signal::derive(move || {
        run_block.get().or_else(|| {
            state
                .sim
                .plan
                .with(|plan| plan.as_ref().is_some_and(|p| p.debug.is_none()))
                .then(|| t!("simulate.debug-blocked"))
        })
    });

    move || {
        state.has_project().then(|| {
            view! {
                <div class="flex items-center gap-0.5">
                    // The file finder, as an icon beside the verbs. Ctrl+P is
                    // the other way in; a search box here was a second field
                    // in front of the finder's own.
                    <button
                        type="button"
                        title=t!("menu.view.quick-open")
                        on:click=move |_| state.layout.quick_open.set(true)
                        class=format!("{BUTTON} text-label-2 hover:text-label")
                    >
                        <IconView icon=Icon::Search size=15 />
                    </button>
                    <span class="mx-1.5 h-4 w-px bg-line" />
                    <button
                        type="button"
                        title=t!("toolbar.build")
                        disabled=move || running.get()
                        on:click=move |_| controller::build_project(state)
                        class=format!("{BUTTON} text-label-2 hover:text-label")
                    >
                        <IconView icon=Icon::Hammer size=15 />
                    </button>
                    // Run becomes Stop in place while something runs — a
                    // build, a run, a debug session — so nothing beside it
                    // moves. A debug session ends through the debugger, which
                    // stops the emulator it booted as well.
                    {move || {
                        if running.get() {
                            view! {
                                <button
                                    type="button"
                                    title=t!("toolbar.stop")
                                    on:click=move |_| {
                                        if state.debug.session.with_untracked(Option::is_some) {
                                            controller::debug_stop(state);
                                        } else {
                                            controller::stop_session_now(state);
                                        }
                                    }
                                    class=format!("{BUTTON} text-crimson")
                                >
                                    <IconView icon=Icon::Stop size=15 />
                                </button>
                            }
                                .into_any()
                        } else {
                            let block = run_block.get();
                            let disabled = block.is_some();
                            let title = block.unwrap_or_else(|| t!("toolbar.run"));
                            view! {
                                <button
                                    type="button"
                                    title=title
                                    disabled=disabled
                                    on:click=move |_| {
                                        state.layout.panel.set("simulate".to_string());
                                        controller::run_simulation(state, false);
                                    }
                                    class=format!("{BUTTON} text-rust")
                                >
                                    <IconView icon=Icon::Play size=15 />
                                </button>
                            }
                                .into_any()
                        }
                    }}
                    {move || {
                        let block = debug_block.get();
                        let disabled = running.get() || block.is_some();
                        let title = block.unwrap_or_else(|| t!("toolbar.debug"));
                        view! {
                            <button
                                type="button"
                                title=title
                                disabled=disabled
                                on:click=move |_| {
                                    state.layout.panel.set("simulate".to_string());
                                    controller::run_simulation(state, true);
                                }
                                class=format!("{BUTTON} text-label-2 hover:text-label")
                            >
                                <IconView icon=Icon::Bug size=15 />
                            </button>
                        }
                    }}
                    <button
                        type="button"
                        title=t!("toolbar.flash")
                        on:click=move |_| state.show_dock(DockTab::Devices)
                        class=format!("{BUTTON} text-label-2 hover:text-label")
                    >
                        <IconView icon=Icon::Flash size=15 />
                    </button>
                </div>
            }
        })
    }
}
