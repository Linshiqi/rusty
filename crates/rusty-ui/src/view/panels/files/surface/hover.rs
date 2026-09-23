//! The hover card: what the server said about the token the pointer
//! settled on, the problem there, and the fixes that hang off it.

use super::*;

/// The card over the token under the pointer. Interactive: long
/// documentation scrolls inside it, and reading is not leaving.
#[component]
pub(super) fn Hover(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        area,
        on_card,
        zoom,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let path = path.clone();
            move || {
                let Some(card) = state.editor.hover.get() else {
                    return ().into_any();
                };
                if card.path != path {
                    return ().into_any();
                }
                let (range, text) = (card.range, card.text);
                let content = state.editor.draft.with_untracked(|draft| {
                    draft
                        .split('\n')
                        .nth(range.start_line as usize)
                        .unwrap_or_default()
                        .to_string()
                });
                let hints = hints_on(state, &path, range.start_line);
                let x = char_left(&content, &hints, range.start_col, zoom.get());
                // Above the token when the token is low in the
                // view — a card clipped by the dock reads as no
                // card at all.
                let place = if pane.opens_up(range.start_line) {
                    let y = 8.0
                        + f64::from(row_for(state, range.start_line)) * row_height(zoom.get())
                        - 4.0;
                    format!("top: {y}px; transform: translateY(-100%)")
                } else {
                    let y = 8.0
                        + f64::from(row_for(state, range.end_line) + 1) * row_height(zoom.get())
                        + 2.0;
                    format!("top: {y}px")
                };
                // The card reads at the editor's own scale: a
                // zoomed-in buffer with an 11px tooltip under it
                // reads as two unrelated programs.
                let font = 11.0 * zoom.get();
                view! {
                    <div
                        class="absolute z-20 max-w-[70ch] overflow-y-auto rounded-[8px] bg-raised px-3 py-2 font-mono leading-relaxed whitespace-pre-wrap shadow-2xl ring-1 ring-line-strong select-text"
                        style=format!(
                            "left: {x}px; {place}; max-height: 40vh; font-size: {font}px",
                        )
                        on:mouseenter=move |_| on_card.set(true)
                        on:mouseleave=move |_| {
                            on_card.set(false);
                            state.editor.hover.set(None);
                        }
                    >
                        {hover_parts(&text)}
                        // What the server offers to do about it,
                        // one click from the pointer that is
                        // already there. Only a squiggle has
                        // these — an `impl Trait for T {}` with
                        // no members is the case they exist for
                        // — and until now the only way to them
                        // was to click into the line and press
                        // Ctrl+. The card is interactive
                        // already, so the buttons cost no new
                        // behaviour: the pointer crossing onto
                        // it keeps it up.
                        {(!card.fixes.fixes.is_empty())
                            .then(|| {
                                let answer = card.fixes.clone();
                                let fix_path = card.path.clone();
                                view! {
                                    <div class="mt-2 flex flex-wrap gap-1.5 border-t border-line pt-2">
                                        {answer
                                            .fixes
                                            .iter()
                                            .enumerate()
                                            .map(|(index, fix)| {
                                                let title = fix.title.clone();
                                                let hint = if fix.elsewhere.is_empty() {
                                                    title.clone()
                                                } else {
                                                    format!(
                                                        "{title} → {}",
                                                        fix.elsewhere.join(", "),
                                                    )
                                                };
                                                let answer = answer.clone();
                                                let fix_path = fix_path.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        title=hint
                                                        on:mousedown=move |
                                                            event: ev::MouseEvent,
                                                        | {
                                                            event.prevent_default();
                                                            event.stop_propagation();
                                                            on_card.set(false);
                                                            state.editor.hover.set(None);
                                                            if let Some(element) =
                                                                area.get_untracked()
                                                            {
                                                                apply_fix(
                                                                    state,
                                                                    &element,
                                                                    &fix_path,
                                                                    &answer,
                                                                    index,
                                                                );
                                                            }
                                                        }
                                                        class="max-w-full truncate rounded-[5px] bg-rust/15 px-2 py-0.5 text-left text-rust hover:bg-rust/25"
                                                    >
                                                        {title}
                                                    </button>
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                }
                            })}
                    </div>
                }
                .into_any()
            }
        }
    }
}
