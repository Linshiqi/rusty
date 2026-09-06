//! The editor pane: whichever document is in front, or the empty state.

use leptos::{ev, html, prelude::*};

use rusty_i18n::t;

use super::*;
use crate::{
    state::{AppState, Divider, Group},
    view::split,
};

/// One editor group — tab strip, header, surface — under a state that
/// addresses `which` group.
///
/// The whole editor was written for one group, reading `state.editor` out of
/// the context. Providing `state.group(which)` here means every component,
/// controller and effect below runs on that group's signals unchanged; what
/// is not below — the tree, the palette, a search hit — asks
/// `state.focused()` which group the user meant, and the group answers that
/// by noting when the pointer or the focus lands in it.
#[component]
pub(super) fn EditorGroup(which: Group) -> impl IntoView {
    let shell = AppState::expect();
    let state = shell.group(which);
    provide_context(state);
    let focus = move || {
        if shell.layout.focus.get_untracked() != which {
            shell.layout.focus.set(which);
        }
    };
    // Each group's share of the width, in permille; alone, a group is the
    // whole width. `flex-basis: 0` so the shares are the only thing that
    // decides, not how long the lines in each happen to be.
    let grow = move || {
        if !shell.layout.split.get() {
            return "flex: 1 1 0px".to_string();
        }
        let left = shell.layout.editor_split.get();
        let share = if which == Group::First {
            left
        } else {
            1000.0 - left
        };
        format!("flex: {share} 1 0px")
    };
    view! {
        <div
            class="flex min-h-0 min-w-0 flex-col"
            style=grow
            on:focusin=move |_| focus()
            on:mousedown=move |_| focus()
        >
            <Editor />
        </div>
    }
}

/// The line between the two groups. Drags in permille of the area it sits
/// in, measured on grab, so half stays half when the window is resized.
#[component]
pub(super) fn SplitGrip(area: NodeRef<html::Div>) -> impl IntoView {
    let state = AppState::expect();
    let on_grab = move |event: ev::MouseEvent| {
        event.prevent_default();
        let per_px = area
            .get_untracked()
            .map(|el| 1000.0 / f64::from(el.client_width()).max(1.0))
            .unwrap_or(1.0);
        split::grab(
            state,
            Divider::EditorSplit,
            f64::from(event.client_x()),
            per_px,
        );
    };
    view! {
        <div
            role="separator"
            aria-orientation="vertical"
            on:mousedown=on_grab
            class=move || {
                let active = state.layout.dragging.get() == Some(Divider::EditorSplit);
                format!(
                    "relative z-10 w-px flex-none cursor-col-resize bg-line transition-colors \
                     before:absolute before:-left-[3px] before:top-0 before:h-full \
                     before:w-[7px] before:content-[''] hover:bg-rust {}",
                    if active { "bg-rust" } else { "" },
                )
            }
        />
    }
}

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

        // One textarea reference for the two components that need it: the
        // surface owns the element, the header's Save formats through it.
        let area: NodeRef<html::Textarea> = NodeRef::new();
        view! {
            <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                <TabStrip />
                <Header document=document.clone() area=area />
                <Surface document=document area=area />
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
