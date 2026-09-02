//! The editor pane: whichever document is in front, or the empty state.

use leptos::prelude::*;

use rusty_i18n::t;

use super::*;
use crate::state::AppState;

#[component]
pub(crate) fn Editor() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let Some(document) = state.editor.document.get() else {
            return view! {
                <div class="flex min-w-0 flex-1 items-center justify-center">
                    <p class="text-callout text-label-3">{t!("files.choose")}</p>
                </div>
            }
            .into_any();
        };

        if document.binary {
            return view! {
                <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                    <TabStrip />
                    <div class="flex flex-1 items-center justify-center px-6 text-center">
                        <p class="max-w-[44ch] text-callout leading-relaxed text-label-2">
                            {t!("files.not-text")}
                        </p>
                    </div>
                </div>
            }
            .into_any();
        }

        // Markdown reads as a page unless asked otherwise. A workbench opens
        // a README to read it far more often than to edit it, and the source
        // is one click away — where the reverse would leave somebody looking
        // at sigils with no clue there was anything else.
        if is_markdown(&document.path)
            && !state
                .editor
                .source_view
                .with(|v| v.contains(&document.path))
        {
            return view! {
                <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                    <TabStrip />
                    <Header document=document.clone() />
                    <div class="min-h-0 flex-1 overflow-y-auto px-6 py-4">
                        // The draft, not the saved text: switching to the page
                        // after an edit must show the edit, or the toggle reads
                        // as having lost it.
                        <div class="mx-auto max-w-[80ch]">
                            {move || {
                                let text = state.editor.draft.get();
                                view! { <crate::view::markdown::Markdown text=text /> }
                            }}
                        </div>
                    </div>
                </div>
            }
            .into_any();
        }

        view! {
            <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                <TabStrip />
                <Header document=document.clone() />
                <Surface document=document />
            </div>
        }
        .into_any()
    }
}

/// Whether this path is Markdown.
///
/// The two extensions in the wild. `.mdown` and friends exist and nobody uses
/// them; a file that is not recognised opens as source, which is wrong in a
/// way the toggle fixes rather than wrong in a way that hides the text.
pub(super) fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}
