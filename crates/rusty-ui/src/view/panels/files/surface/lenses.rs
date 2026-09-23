//! `▶ Run Test | Debug` beside every test and every module holding one,
//! where VS Code puts it (`lens.rs` places it).

use super::*;

/// The lenses over the tests in the window.
#[component]
pub(super) fn Lenses(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        zoom,
        window,
        runnables,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let lens_path = path.clone();
            move || {
            let found = runnables.get();
            if found.is_empty() {
                return ().into_any();
            }
            let draft = state.editor.draft.get();
            let lines: Vec<&str> = draft.lines().collect();
            let z = zoom.get();
            let height = row_height(z);
            let icon_px = (height * 0.6).round().max(7.0) as u32;
            let range = window.get();
            found
                .into_iter()
                .filter_map(|r| {
                    let line = lens_line(&lines, r.line)?;
                    // Inside a collapsed region the header stands
                    // for the line, and a stack of lenses on one
                    // header would say nothing readable.
                    let row = row_for(state, line);
                    if line_of_row(state, row) != line || !range.contains(&row) {
                        return None;
                    }
                    // After everything drawn on the line, hints too.
                    let content = lines.get(line as usize).copied().unwrap_or_default();
                    let x = line_right(content, &hints_on(state, &lens_path, line), z)
                        + line_px("  ") * z;
                    let y = row_top(state, line, z);
                    let (run_label, run_title) = match r.kind {
                        rusty_edit::RunnableKind::Module => (
                            t!("files.lens-run-tests"),
                            t!("files.run-module", name = r.name.clone()),
                        ),
                        rusty_edit::RunnableKind::Test => (
                            t!("files.lens-run-test"),
                            t!("files.run-test", name = r.name.clone()),
                        ),
                    };
                    let debug_title = t!("files.debug-test", name = r.name.clone());
                    let run_filter = r.filter.clone();
                    let debug_filter = r.filter.clone();
                    Some(view! {
                        <div
                            class="pointer-events-none absolute z-10 flex items-center gap-2 font-sans text-footnote leading-none text-label-3 select-none"
                            style=format!("left: {x}px; top: {y}px; height: {height}px")
                        >
                            <button
                                type="button"
                                title=run_title
                                // The editor keeps focus: a lens is a
                                // command, not a place to be.
                                on:mousedown=|event: ev::MouseEvent| event.prevent_default()
                                on:click=move |event: ev::MouseEvent| {
                                    event.stop_propagation();
                                    controller::run_test(state, run_filter.clone());
                                }
                                class="pointer-events-auto flex items-center gap-1 hover:text-label"
                            >
                                <IconView icon=Icon::Play size=icon_px />
                                {run_label}
                            </button>
                            <span class="text-label-4">"|"</span>
                            <button
                                type="button"
                                title=debug_title
                                on:mousedown=|event: ev::MouseEvent| event.prevent_default()
                                on:click=move |event: ev::MouseEvent| {
                                    event.stop_propagation();
                                    controller::debug_test(state, debug_filter.clone());
                                }
                                class="pointer-events-auto flex items-center gap-1 hover:text-label"
                            >
                                <IconView icon=Icon::Bug size=icon_px />
                                {t!("files.lens-debug")}
                            </button>
                        </div>
                    })
                })
                .collect_view()
                .into_any()
        }}
    }
}
