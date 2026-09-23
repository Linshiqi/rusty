//! The quick-fix popup: what rust-analyzer would fix at the caret.

use super::*;

/// The fixes offered at the caret, anchored under its line.
#[component]
pub(super) fn QuickFixes(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        area,
        zoom,
        picked_action,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let path = path.clone();
            move || {
                let Some((for_path, line, answer)) = state.editor.actions.get()
                else {
                    return ().into_any();
                };
                if for_path != path {
                    return ().into_any();
                }
                let fixes = answer.fixes;
                let chosen = picked_action.get().min(fixes.len().saturating_sub(1));
                let place = card_place(state, line, zoom.get(), pane.opens_up(line));
                view! {
                    <div
                        class="absolute z-20 min-w-[280px] rounded-[8px] bg-raised py-1 font-mono text-footnote shadow-2xl ring-1 ring-line-strong"
                        style=format!("left: 48px; {place}")
                    >
                        {fixes
                            .into_iter()
                            .enumerate()
                            .map(|(index, fix)| {
                                let selected = index == chosen;
                                // What else it changes, or its kind:
                                // a fix that writes another file
                                // says which before it is taken.
                                let kind = if fix.elsewhere.is_empty() {
                                    fix.kind.clone().unwrap_or_default()
                                } else {
                                    format!("→ {}", fix.elsewhere.join(", "))
                                };
                                view! {
                                    <button
                                        type="button"
                                        on:mousedown=move |event: ev::MouseEvent| {
                                            event.prevent_default();
                                            event.stop_propagation();
                                            if let Some(element) =
                                                area.get_untracked()
                                            {
                                                apply_action(
                                                    state, &element, index,
                                                );
                                            }
                                        }
                                        class=if selected {
                                            "flex w-full items-baseline gap-2 bg-selection px-2.5 py-0.5 text-left text-rust"
                                        } else {
                                            "flex w-full items-baseline gap-2 px-2.5 py-0.5 text-left text-label-2"
                                        }
                                    >
                                        <span class="shrink-0">{fix.title.clone()}</span>
                                        <span class="min-w-0 flex-1 truncate text-right text-label-3">
                                            {kind}
                                        </span>
                                    </button>
                                }
                            })
                            .collect_view()}
                    </div>
                }
                .into_any()
            }
        }
    }
}
