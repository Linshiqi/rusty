//! The signature card, floated above the line whose call it describes.

use super::*;

/// The call the caret is inside, with its active parameter lit.
#[component]
pub(super) fn SignatureCard(pane: Pane) -> impl IntoView {
    let Pane { state, zoom, .. } = pane;
    let path = pane.path.get_value();
    view! {
            {
                let path = path.clone();
                move || {
                    let Some((for_path, line, info)) = state.editor.signature.get() else {
                        return ().into_any();
                    };
                    if for_path != path {
                        return ().into_any();
                    }
    // Above by nature — it describes the call being typed — but
                    // near the top of the view "above" is off screen,
                    // so it flips below the line there.
                    let near_top = pane.line_in_view(line).is_some_and(|(top, _)| top < 96.0);
                    let place = card_place(state, line, zoom.get(), !near_top);
                    let label = info.label;
                    let split = match (info.param_start, info.param_end) {
                        (Some(start), Some(end)) => {
                            let start = start as usize;
                            let end = (end as usize).min(label.len());
                            if start <= end
                                && label.is_char_boundary(start)
                                && label.is_char_boundary(end)
                            {
                                Some((start, end))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    let (before, active, after) = match split {
                        Some((start, end)) => (
                            label[..start].to_string(),
                            label[start..end].to_string(),
                            label[end..].to_string(),
                        ),
                        None => (label, String::new(), String::new()),
                    };
                    // One line of docs, not the essay — hover exists.
                    let doc = info
                        .doc
                        .as_deref()
                        .and_then(|d| d.lines().find(|l| !l.trim().is_empty()))
                        .map(str::to_string);
                    view! {
                        <div
                            class="absolute z-10 max-w-[76ch] rounded-[8px] bg-raised px-3 py-1.5 font-mono text-footnote shadow-xl ring-1 ring-line-strong"
                            style=format!(
                                "left: 8px; {place}",
                            )
                        >
                            <div class="whitespace-pre-wrap select-text">
                                <span class="text-label-2">{before}</span>
                                <span class="font-semibold text-rust">{active}</span>
                                <span class="text-label-2">{after}</span>
                            </div>
                            {doc
                                .map(|text| {
                                    view! {
                                        <div class="mt-0.5 max-w-[70ch] truncate font-sans text-caption text-label-3">
                                            {text}
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
