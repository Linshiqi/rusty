//! Modal editing and text size.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{Button, ButtonKind},
};

use super::*;

/// The editor itself, rather than how it looks.
///
/// Modal editing was reachable only from a menu at first, which is where a
/// setting is *looked* for last. Anything that changes how the editor
/// behaves belongs here, where somebody goes to ask "can it do X".
#[component]
pub(super) fn EditorSettings() -> impl IntoView {
    let state = AppState::expect();
    let step = move |by: f64| {
        let (min, max) = crate::state::EDITOR_ZOOM_RANGE;
        let next = if by == 0.0 {
            1.0
        } else {
            (state.editor.zoom.get_untracked() + by * 0.1).clamp(min, max)
        };
        state.editor.zoom.set(next);
        crate::state::remember_zoom(next);
    };

    view! {
        <Group footer=t!("settings.editor.text-size-note")>
            <Row label=t!("settings.editor.vim") detail=t!("settings.editor.vim-detail")>
                <Switch
                    on=Signal::derive(move || state.editor.vim_on.get())
                    on_toggle=Callback::new(move |on| controller::set_vim(state, on))
                />
            </Row>
            <Row label=t!("settings.editor.text-size")>
                <div class="inline-flex items-center rounded-[7px] bg-sunken p-0.5">
                    <button
                        type="button"
                        class="h-[24px] rounded-[5px] px-2.5 text-callout text-label-2 hover:bg-content hover:text-label"
                        on:click=move |_| step(-1.0)
                    >
                        "A−"
                    </button>
                    <span class="tnum w-[5ch] text-center font-mono text-callout text-label">
                        {move || format!("{:.0}%", state.editor.zoom.get() * 100.0)}
                    </span>
                    <button
                        type="button"
                        class="h-[24px] rounded-[5px] px-2.5 text-callout text-label-2 hover:bg-content hover:text-label"
                        on:click=move |_| step(1.0)
                    >
                        "A+"
                    </button>
                </div>
                <Button
                    label=t!("settings.editor.reset")
                    kind=ButtonKind::Quiet
                    on_click=Callback::new(move |_| step(0.0))
                />
            </Row>
        </Group>
    }
}
