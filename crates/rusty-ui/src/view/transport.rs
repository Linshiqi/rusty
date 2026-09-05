//! The debugger's transport controls.
//!
//! A pill floating over the working area while a session is live — VS Code's
//! debug toolbar — and nothing at all otherwise. Floating rather than in a
//! panel's toolbar, because the panel you are on when you need them is not
//! fixed: pressing Debug moves you to Simulate to watch the board, a
//! breakpoint moves you to the editor to read the line, and both places have
//! to be able to continue, step and stop. They used to be copied into the
//! editor's rail and the board's, and their arrival pushed Run down the
//! column; an overlay is one copy, and its arrival moves nothing.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::icon::{Icon, IconView},
};

const BUTTON: &str = "grid size-7 place-items-center rounded-[6px] transition-colors \
                      hover:bg-sunken disabled:pointer-events-none disabled:opacity-35";

/// Continue/pause, the three steps, and stop — or nothing at all when no
/// session is live.
#[component]
pub fn DebugTransport() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let debug = state.debug.session.get()?;
        let running = debug.running;
        let step = move |action: &'static str, icon, title: String| {
            view! {
                <button
                    type="button"
                    title=title
                    disabled=running
                    on:click=move |_| controller::debug_control(state, action)
                    class=format!("{BUTTON} text-label-2 hover:text-label")
                >
                    <IconView icon=icon size=15 />
                </button>
            }
        };
        Some(view! {
            <div class="flex items-center gap-0.5 rounded-[8px] bg-raised p-0.5 shadow-xl ring-1 ring-line-strong">
                {if running {
                    view! {
                        <button
                            type="button"
                            title=t!("debugger.pause")
                            on:click=move |_| controller::debug_control(state, "pause")
                            class=format!("{BUTTON} text-amber")
                        >
                            <IconView icon=Icon::Pause size=15 />
                        </button>
                    }
                        .into_any()
                } else {
                    view! {
                        <button
                            type="button"
                            title=t!("debugger.continue")
                            on:click=move |_| controller::debug_control(state, "resume")
                            class=format!("{BUTTON} text-patina")
                        >
                            <IconView icon=Icon::Play size=15 />
                        </button>
                    }
                        .into_any()
                }}
                {step("over", Icon::StepOver, t!("debugger.step-over"))}
                {step("into", Icon::StepInto, t!("debugger.step-into"))}
                {step("out", Icon::StepOut, t!("debugger.step-out"))}
                <span class="mx-0.5 h-4 w-px bg-line" />
                // Named for what it actually does. The debug run is what booted
                // the target, so stopping it stops that too — the alternative was
                // an orphaned QEMU nothing in the window could reach.
                <button
                    type="button"
                    title=t!("debugger.stop")
                    on:click=move |_| controller::debug_stop(state)
                    class=format!("{BUTTON} text-crimson")
                >
                    <IconView icon=Icon::Stop size=15 />
                </button>
            </div>
        })
    }
}
