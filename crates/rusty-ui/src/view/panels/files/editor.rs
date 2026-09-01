//! The editor pane: whichever document is in front, or the empty state.

use leptos::prelude::*;

use super::*;
use crate::state::AppState;

#[component]
pub(crate) fn Editor() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let Some(document) = state.editor.document.get() else {
            return view! {
                <div class="flex min-w-0 flex-1 items-center justify-center">
                    <p class="text-callout text-label-3">"Choose a file."</p>
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
                            "This is not a text file. rusty will not render a firmware image as \
                             characters — the result is noise, and for a large one it would take \
                             the window down with it."
                        </p>
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
