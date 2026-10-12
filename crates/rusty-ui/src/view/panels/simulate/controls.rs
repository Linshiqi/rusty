//! The sheet's own controls, in its corner as every schematic editor keeps
//! them: Save once there is something to save, import and export, undo and
//! redo, tidy, pause, zoom, fit, the snap grid — and beside the editor, the
//! library's `+` and the pane's own two.

use super::*;

/// The corner cluster. Pointer events stop here, or a press on Zoom would
/// also be a press on the sheet under it.
#[component]
pub(super) fn SheetControls(board: Board) -> impl IntoView {
    let Board {
        state,
        compact,
        running,
        live,
        library_open,
        rows,
        parts,
        wires,
        dirty,
        selected,
        selected_wire,
        grid,
        marked,
        view,
        no_connect,
        history,
        future,
        save,
        ..
    } = board;
    view! {
        <div
            class="absolute top-2 right-2 z-20 flex items-center gap-0.5 rounded-[8px] bg-raised p-0.5 ring-1 ring-line-strong"
            on:pointerdown=move |event: ev::PointerEvent| event.stop_propagation()
            on:contextmenu=move |event: ev::MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
            }
        >
            // Beside the editor the library is behind this, as
            // Wokwi's parts are behind its `+`.
            {compact
                .then(|| {
                    view! {
                        <button
                            type="button"
                            title=t!("simulate.add-part")
                            disabled=move || live.get()
                            on:click=move |_| library_open.update(|open| *open = !*open)
                            class=move || {
                                if library_open.get() {
                                    format!("{SHEET_BUTTON} bg-selection text-rust")
                                } else {
                                    SHEET_BUTTON.to_string()
                                }
                            }
                        >
                            <IconView icon=Icon::Plus size=14 />
                        </button>
                        <span class="mx-0.5 h-4 w-px bg-line" />
                    }
                })}
            <button
                type="button"
                title=t!("simulate.save")
                disabled=move || !dirty.get() || live.get()
                on:click=move |_| save.run(())
                class=SHEET_BUTTON
            >
                <IconView icon=Icon::Save size=14 />
            </button>
            // KiCad, both ways. Beside Save because that is what
            // they are — the same sheet, written somewhere else.
            // Not beside the editor, where the corner is a
            // column wide; the panel has them.
            <span class=move || {
                if compact { "hidden" } else { "mx-0.5 h-4 w-px bg-line" }
            } />
            <button
                type="button"
                title=t!("simulate.schematic-import")
                class:hidden=compact
                disabled=move || live.get()
                on:click=move |_| {
                    controller::import_schematic(
                        state,
                        Callback::new(move |brought: Sheet| {
                            board.checkpoint();
                            let rows = rows.get_untracked();
                            // Laid out on arrival: another
                            // editor's canvas is not this one,
                            // and a diagram's own coordinates
                            // land a dozen parts in one square
                            // inch here — which reads as an
                            // import that lost half of them.
                            let mut brought_parts = parts_of(&brought, &rows);
                            let mut brought_wires = brought.wires.clone();
                            layout::arrange(&mut brought_parts, &mut brought_wires);
                            parts.set(brought_parts);
                            wires.set(brought_wires);
                            no_connect.set(brought.no_connect.clone());
                            marked.set(Vec::new());
                            selected.set(None);
                            selected_wire.set(None);
                            dirty.set(true);
                        }),
                    )
                }
                class=SHEET_BUTTON
            >
                "⭳"
            </button>
            <button
                type="button"
                title=t!("simulate.kicad-export")
                class:hidden=compact
                on:click=move |_| {
                    let sheet = board.sheet_now();
                    controller::export_kicad(state, sheet);
                }
                class=SHEET_BUTTON
            >
                "⭱"
            </button>
            <span class="mx-0.5 h-4 w-px bg-line" />
            <button
                type="button"
                title=t!("simulate.undo")
                disabled=move || history.with(Vec::is_empty) || live.get()
                on:click=move |_| board.undo()
                class=SHEET_BUTTON
            >
                "↶"
            </button>
            <button
                type="button"
                title=t!("simulate.redo")
                disabled=move || future.with(Vec::is_empty) || live.get()
                on:click=move |_| board.redo()
                class=SHEET_BUTTON
            >
                "↷"
            </button>
            // Lay the whole sheet out again. One undo step, and
            // only when asked: a board somebody arranged by hand
            // is theirs, and a rule that tidied on its own would
            // move their work out from under them.
            <button
                type="button"
                title=t!("simulate.tidy")
                class:hidden=compact
                disabled=move || live.get()
                on:click=move |_| {
                    board.checkpoint();
                    parts.update(|list| {
                        wires.update(|w| layout::arrange(list, w));
                    });
                    dirty.set(true);
                }
                class=SHEET_BUTTON
            >
                "⌗"
            </button>
            // Only while something is running: a Pause on a
            // sheet with no emulator behind it is a button that
            // can only refuse.
            {move || {
                running
                    .get()
                    .then(|| {
                        let paused = state.sim.paused;
                        view! {
                            <span class="mx-0.5 h-4 w-px bg-line" />
                            <button
                                type="button"
                                title=move || {
                                    if paused.get() {
                                        t!("simulate.resume")
                                    } else {
                                        t!("simulate.pause")
                                    }
                                }
                                on:click=move |_| {
                                    controller::sim_pause(state, !paused.get_untracked())
                                }
                                class=SHEET_BUTTON
                            >
                                {move || if paused.get() { "▶" } else { "❚❚" }}
                            </button>
                        }
                    })
            }}
            <span class="mx-0.5 h-4 w-px bg-line" />
            <button
                type="button"
                title=t!("simulate.zoom-out")
                on:click=move |_| {
                    view.update(|(_, _, k)| *k = (*k / 1.2).max(CANVAS_ZOOM_RANGE.0))
                }
                class=SHEET_BUTTON
            >
                "−"
            </button>
            <span class="min-w-[5ch] text-center font-mono text-footnote text-label-3">
                {move || format!("{:.0}%", view.get().2 * 100.0)}
            </span>
            <button
                type="button"
                title=t!("simulate.zoom-in")
                on:click=move |_| {
                    view.update(|(_, _, k)| *k = (*k * 1.2).min(CANVAS_ZOOM_RANGE.1))
                }
                class=SHEET_BUTTON
            >
                "+"
            </button>
            <button
                type="button"
                title=t!("simulate.fit")
                on:click=move |_| board.fit_view()
                class=SHEET_BUTTON
            >
                <IconView icon=Icon::Fit size=14 />
            </button>
            <button
                type="button"
                title=t!("simulate.grid")
                class:hidden=compact
                on:click=move |_| {
                    grid.update(|g| {
                        *g = match *g as i32 {
                            1 => 4.0,
                            4 => 8.0,
                            8 => 16.0,
                            _ => 1.0,
                        }
                    })
                }
                class="flex h-7 items-center gap-1 rounded-[6px] px-1.5 font-mono text-caption text-label-2 hover:bg-sunken hover:text-label"
            >
                <IconView icon=Icon::Grid size=13 />
                <span class="tnum leading-none">
                    {move || format!("{}", grid.get() as i32)}
                </span>
            </button>
            // The pane's own two: the whole editor, and away.
            {compact
                .then(|| {
                    view! {
                        <span class="mx-0.5 h-4 w-px bg-line" />
                        <button
                            type="button"
                            title=t!("simulate.open-panel")
                            on:click=move |_| state.layout.panel.set("simulate".to_string())
                            class=SHEET_BUTTON
                        >
                            <IconView icon=Icon::External size=14 />
                        </button>
                        <button
                            type="button"
                            title=t!("simulate.hide-board")
                            on:click=move |_| state.layout.board_beside.set(false)
                            class=SHEET_BUTTON
                        >
                            <IconView icon=Icon::Close size=14 />
                        </button>
                    }
                })}
        </div>
    }
}
