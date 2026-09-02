//! The debugger's transport controls.
//!
//! Shared rather than owned by one panel, because the panel you are on when
//! you need them is not fixed: pressing Debug moves you to Simulate to watch
//! the board, a breakpoint moves you to the editor to read the line, and both
//! places have to be able to continue, step and stop. They lived only on the
//! editor's toolbar, so a debug session started from the board view could not
//! be stopped without leaving the board view.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::icon::{Icon, IconView},
};

/// Continue/pause, the three steps, and stop — or nothing at all when no
/// session is live.
#[component]
pub fn DebugTransport() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let debug = state.debug.session.get()?;
        let running = debug.running;
        let step = move |action: &'static str, icon, title: &'static str| {
            view! {
                <button
                    type="button"
                    title=title
                    disabled=running
                    on:click=move |_| controller::debug_control(state, action)
                    class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-35"
                >
                    <IconView icon=icon size=15 />
                </button>
            }
        };
        Some(view! {
            <span class="my-1 h-px w-5 bg-line" />
            {if running {
                view! {
                    <button
                        type="button"
                        title=t!("debugger.pause")
                        on:click=move |_| controller::debug_control(state, "pause")
                        class="grid size-8 place-items-center rounded-[6px] text-amber hover:bg-sunken"
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
                        class="grid size-8 place-items-center rounded-[6px] text-patina hover:bg-sunken"
                    >
                        <IconView icon=Icon::Play size=15 />
                    </button>
                }
                    .into_any()
            }}
            {step("over", Icon::StepOver, "Step over (F10)")}
            {step("into", Icon::StepInto, "Step into (F11)")}
            {step("out", Icon::StepOut, "Step out (Shift+F11)")}
            // Named for what it actually does. The debug run is what booted
            // the target, so stopping it stops that too — the alternative was
            // an orphaned QEMU nothing in the window could reach.
            <button
                type="button"
                title=t!("debugger.stop")
                on:click=move |_| controller::debug_stop(state)
                class="grid size-8 place-items-center rounded-[6px] text-crimson hover:bg-sunken"
            >
                <IconView icon=Icon::Stop size=15 />
            </button>
        })
    }
}
