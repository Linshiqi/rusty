//! The completion popup, anchored under the word it is completing.

use super::*;

/// The completion popup: the server's answer for the word being typed,
/// ranked and filtered as the word grows.
#[component]
pub(super) fn Completions(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        area,
        zoom,
        picked,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let path = path.clone();
            move || {
                let Some(popup) = state.editor.completion.get() else {
                    return ().into_any();
                };
                if popup.path != path {
                    return ().into_any();
                }
                let draft = state.editor.draft.get();
                let shown: Vec<(usize, CompletionItem)> = visible_items(&popup, &draft);
                if shown.is_empty() {
                    return ().into_any();
                }
                let chosen = picked.get().min(shown.len() - 1);
                let content = draft.split('\n').nth(popup.line as usize).unwrap_or_default();
                let hints = hints_on(state, &path, popup.line);
                let x = char_left(content, &hints, popup.word_start, zoom.get());
                let place =
                    card_place(state, popup.line, zoom.get(), pane.opens_up(popup.line));
                // A window around the selection rather than a
                // scrollbar: nine rows is what the eye takes in,
                // and the arrows walk the rest into view.
                let from = chosen.saturating_sub(4).min(shown.len().saturating_sub(9));
                view! {
                    <div
                        class="absolute z-20 min-w-[260px] rounded-[8px] bg-raised py-1 font-mono text-footnote shadow-2xl ring-1 ring-line-strong"
                        style=format!("left: {x}px; {place}")
                    >
                        {shown
                            .into_iter()
                            .skip(from)
                            .take(9)
                            .map(|(index, item)| {
                                let selected = index == chosen;
                                let kind = item.kind.clone().unwrap_or_default();
                                // The type or signature, as the
                                // server shows it beside the name.
                                let detail = item
                                    .description
                                    .clone()
                                    .or_else(|| item.detail.clone())
                                    .unwrap_or_default();
                                view! {
                                    <button
                                        type="button"
                                        on:mousedown=move |event: ev::MouseEvent| {
                                            // Before the textarea's
                                            // own mousedown closes
                                            // the popup.
                                            event.prevent_default();
                                            event.stop_propagation();
                                            if let Some(element) =
                                                area.get_untracked()
                                            {
                                                accept_completion(
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
                                        <span class="w-[7ch] shrink-0 truncate text-label-3">
                                            {kind}
                                        </span>
                                        <span class="shrink-0">{item.label.clone()}</span>
                                        <span class="shrink-0 text-label-4">
                                            {item.label_detail.clone().unwrap_or_default()}
                                        </span>
                                        <span class="min-w-0 flex-1 truncate text-label-3">
                                            {detail}
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
