//! The margin: line numbers, breakpoint dots and fold chevrons, for the
//! rows in the window.

use super::*;

/// The margin beside the text.
#[component]
pub(super) fn Gutter(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        rows_total,
        window,
        gutter_style,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        <div
            class="flex-none py-2 pr-2 pl-3 text-right text-label-4 select-none"
            style=gutter_style
        >
            <div style=move || pane.spacer(window.get().start)></div>
            <For
                each=move || pane.gutter_rows()
                key=|row| *row
                children={
                    let path = path.clone();
                    move |row: GutterRow| {
                        let GutterRow { line, chevron, collapsed, folds_column, icon_px } = row;
                        let n = line + 1;
                        let file = path.clone();
                        let toggle = file.clone();
                        let marked = Signal::derive(move || {
                            state.debug.breakpoints.with(|list| {
                                list.iter().any(|(f, l)| f == &file && *l == line)
                            })
                        });
                        // Fold control. Shown only where something can
                        // collapse, and only on hover unless it is
                        // already folded — a chevron on every second
                        // line is a margin nobody can read past.
                        let chevron = chevron.then(|| {
                            // VSCode's shape: a stroked chevron, down
                            // when the region is open and turned a
                            // quarter right when it is collapsed. A
                            // filled triangle reads as a disclosure
                            // widget from a different decade and,
                            // worse, as the run arrow's sibling rather
                            // than as a different kind of control.
                            let class = if collapsed {
                                "flex shrink-0 -rotate-90 items-center text-label-2"
                            } else {
                                "flex shrink-0 items-center text-transparent \
                                 group-hover:text-label-3"
                            };
                            let title = if collapsed {
                                t!("files.unfold")
                            } else {
                                t!("files.fold")
                            };
                            view! {
                                <button
                                    type="button"
                                    title=title
                                    on:click=move |event: ev::MouseEvent| {
                                        event.stop_propagation();
                                        toggle_fold(state, line);
                                    }
                                    class=class
                                >
                                    <IconView icon=Icon::Chevron size=icon_px />
                                </button>
                            }
                        });
                        // Each decoration gets a slot of its own on
                        // *every* row, occupied or not. The row is
                        // `justify-end`, so a line with no chevron lets
                        // its number slide right into the chevron's
                        // place — and one number out of step with its
                        // neighbours reads as the gutter having lost
                        // track of the file.
                        let slot = format!("width: {icon_px}px");
                        view! {
                            // The dot sits *left of* the number, as every
                            // editor with a breakpoint margin puts it:
                            // replacing the number meant setting a
                            // breakpoint cost you the line you were on.
                            // Each number is a breakpoint target, as in
                            // every debugger since the first one with a
                            // mouse, and keeps its real number: a folded
                            // file whose numbers renumbered themselves
                            // would make every compiler error point at
                            // the wrong place.
                            <div
                                on:click=move |_| {
                                    controller::debug_breakpoint(state, toggle.clone(), line)
                                }
                                title=t!("files.breakpoint")
                                class="group flex cursor-pointer items-center justify-end gap-1.5"
                            >
                                <span class=move || {
                                    if marked.get() {
                                        "text-crimson"
                                    } else {
                                        // Faint under the pointer, invisible
                                        // otherwise: a margin that looks
                                        // inert is a margin nobody clicks.
                                        "text-transparent group-hover:text-crimson/50"
                                    }
                                }>
                                    "●"
                                </span>
                                <span>{n.to_string()}</span>
                                // Right of the number, hard against the
                                // code, which is where VSCode puts it —
                                // the chevron belongs to the line it
                                // opens, and on the far side of the
                                // margin it reads as another breakpoint
                                // control.
                                {folds_column
                                    .then(|| {
                                        view! {
                                            <span
                                                class="flex shrink-0 items-center justify-center"
                                                style=slot
                                            >
                                                {chevron}
                                            </span>
                                        }
                                    })}
                            </div>
                        }
                    }
                }
            />
            <div style=move || pane.spacer(rows_total.get().saturating_sub(window.get().end))></div>
        </div>
    }
}
