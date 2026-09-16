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

    view! {
        <Group footer=t!("settings.editor.text-size-note")>
            <Row label=t!("settings.editor.vim") detail=t!("settings.editor.vim-detail")>
                <Switch
                    on=Signal::derive(move || state.editor.vim_on.get())
                    on_toggle=Callback::new(move |on| controller::set_vim(state, on))
                />
            </Row>
            <Row
                label=t!("settings.editor.auto-save")
                detail=t!("settings.editor.auto-save-detail")
            >
                <Switch
                    on=Signal::derive(move || state.editor.auto_save.get())
                    on_toggle=Callback::new(move |on| controller::set_auto_save(state, on))
                />
            </Row>
            <Row label=t!("settings.editor.text-size")>
                <ZoomStepper
                    zoom=state.editor.zoom
                    range=crate::state::EDITOR_ZOOM_RANGE
                    remember=crate::state::remember_zoom
                />
            </Row>
            // The page's own, because prose and a listing are read at
            // different sizes; Ctrl+wheel over a page moves the same number.
            <Row label=t!("settings.editor.page-size")>
                <ZoomStepper
                    zoom=state.editor.page_zoom
                    range=crate::state::PAGE_ZOOM_RANGE
                    remember=crate::state::remember_page_zoom
                />
            </Row>
        </Group>

        // Its own group, because it is not about how the editor behaves: it
        // chooses the program behind completion and navigation, and the
        // footer says what it is for so the field does not read as a knob
        // everybody should turn.
        <Group footer=t!("settings.editor.analyzer-note")>
            <Row
                label=t!("settings.editor.analyzer")
                detail=t!("settings.editor.analyzer-detail")
                stacked=true
            >
                <TextField
                    value=Signal::derive(move || state.editor.rust_analyzer.get())
                    on_input=Callback::new(move |path| {
                        controller::set_rust_analyzer(state, path)
                    })
                    placeholder=t!("settings.editor.analyzer-auto")
                    width="w-full"
                />
            </Row>
        </Group>
    }
}

/// `A−  100%  A+  Reset` over one zoom factor: a tenth per press, clamped
/// to the range, remembered through the given door.
#[component]
fn ZoomStepper(zoom: RwSignal<f64>, range: (f64, f64), remember: fn(f64)) -> impl IntoView {
    let step = move |by: f64| {
        let (min, max) = range;
        let next = if by == 0.0 {
            1.0
        } else {
            (zoom.get_untracked() + by * 0.1).clamp(min, max)
        };
        zoom.set(next);
        remember(next);
    };
    view! {
        <div class="inline-flex items-center rounded-[7px] bg-sunken p-0.5">
            <button
                type="button"
                class="h-[24px] rounded-[5px] px-2.5 text-callout text-label-2 hover:bg-content hover:text-label"
                on:click=move |_| step(-1.0)
            >
                "A−"
            </button>
            <span class="tnum w-[5ch] text-center font-mono text-callout text-label">
                {move || format!("{:.0}%", zoom.get() * 100.0)}
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
    }
}
