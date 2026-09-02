//! Serial ports and debug probes currently attached.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, Dot, MenuItem, Tone},
};

#[component]
pub(super) fn DevicesTab() -> impl IntoView {
    let state = AppState::expect();
    let menu = RwSignal::new(None::<(f64, f64)>);

    // One list of what is plugged in, one place to act on it.
    view! {
        <div
            class="min-h-0 flex-1 overflow-y-auto pb-2"
            on:contextmenu=move |event: leptos::ev::MouseEvent| {
                event.prevent_default();
                menu.set(Some((event.client_x() as f64, event.client_y() as f64)));
            }
        >
            // The whole device workspace — list, mode toggle, command, run.
            // It lived in a Flash panel once; every path to it was a detour
            // past this list, which is where the eye already was.
            <crate::view::panels::session::Session />
            <div class="mt-1 flex items-center gap-2 px-4">
                <Dot tone=Tone::Neutral />
                <span class="text-footnote text-label-3">
                    {move || {
                        format!(
                            "named against {} boards and {} chips",
                            state.project.boards.with(Vec::len),
                            state.project.chips.with(Vec::len),
                        )
                    }}
                </span>
                <button
                    type="button"
                    class="rounded-[5px] px-2 py-0.5 text-footnote text-rust hover:underline"
                    on:click=move |_| controller::load_catalog(state)
                >
                    {t!("context.devices-reload")}
                </button>
            </div>
        </div>

        {move || {
            let (x, y) = menu.get()?;
            let close = Callback::new(move |_| menu.set(None));
            Some(
                view! {
                    <ContextMenu x=x y=y on_close=close>
                        <MenuItem
                            label=t!("context.devices-rescan")
                            on_select=Callback::new(move |_| {
                                controller::scan_devices(state);
                                menu.set(None);
                            })
                        />
                        <MenuItem
                            label=t!("context.devices-reload")
                            on_select=Callback::new(move |_| {
                                controller::load_catalog(state);
                                menu.set(None);
                            })
                        />
                    </ContextMenu>
                },
            )
        }}
    }
}
