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

        // A picture reads as a picture. An SVG is text as well, and its
        // source is one click away, as a Markdown page's is; a PNG has no
        // source to show and arrives `binary`, so the picture is all there
        // is of it — and better than a notice that it is not text.
        if is_picture(&document.path)
            && !state
                .editor
                .source_view
                .with(|v| v.contains(&document.path))
        {
            let header = (!document.binary).then(|| view! { <Header document=document.clone() /> });
            return view! {
                <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                    <TabStrip />
                    {header}
                    <Picture path=document.path.clone() from_draft=!document.binary />
                </div>
            }
            .into_any();
        }

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
                    <Scroller path=document.path.clone() class="min-h-0 flex-1 overflow-y-auto px-6 py-4">
                        // The draft, not the saved text: switching to the page
                        // after an edit must show the edit, or the toggle reads
                        // as having lost it.
                        <div class="mx-auto max-w-[80ch]">
                            {move || {
                                let text = state.editor.draft.get();
                                // The file's own path is what its figures
                                // are relative to.
                                let base = state.active_path_now();
                                view! { <crate::view::markdown::Markdown text=text base=base /> }
                            }}
                        </div>
                    </Scroller>
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

/// Whether this path is an image the WebView can draw. One list, the git
/// panel's, so a file compared as a picture in a diff opens as one here.
pub(super) fn is_picture(path: &str) -> bool {
    rusty_git::image_mime(path).is_some()
}

/// The one picture format that is also text, and so has a source to show.
pub(super) fn is_svg(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".svg")
}

/// The picture a file is.
///
/// An SVG is drawn from the *draft*, so an edit made in the source view shows
/// the moment the toggle flips back, and the picture can never be a stale
/// copy of the text beside it — the Markdown page's rule. A binary image has
/// no draft and comes through the same fetch the page view's figures use.
#[component]
fn Picture(path: String, from_draft: bool) -> impl IntoView {
    let state = AppState::expect();
    if !from_draft {
        crate::controller::load_image(state, path.clone());
    }
    let key = path.clone();
    let src = move || -> Result<String, String> {
        if from_draft {
            let svg = state.editor.draft.get();
            let encoded: String = js_sys::encode_uri_component(&svg).into();
            return Ok(format!("data:image/svg+xml;charset=utf-8,{encoded}"));
        }
        match state.editor.images.with(|images| images.get(&key).cloned()) {
            Some(crate::state::ImageLoad::Ready(url)) => Ok(url),
            Some(crate::state::ImageLoad::Failed(error)) => Err(error),
            _ => Err(t!("image.loading")),
        }
    };
    let at = path.clone();
    view! {
        <Scroller path=at class="flex min-h-0 flex-1 items-center justify-center overflow-auto p-6">
            {move || match src() {
                Ok(url) => view! {
                    <img src=url alt=path.clone() class="max-h-full max-w-full object-contain" />
                }
                    .into_any(),
                Err(message) => view! {
                    <p class="max-w-[44ch] text-center text-callout leading-relaxed text-label-3">
                        {message}
                    </p>
                }
                    .into_any(),
            }}
        </Scroller>
    }
}

/// The scroller under a page or a picture, which puts a fronted tab back
/// where it was left.
///
/// The element is one for every document that passes through it — Leptos
/// rebuilds the view in place — so a switch left the new document at the old
/// one's offset, and a chapter you had read half of opened at the top when
/// you came back. The code surface does the same for itself, because it has
/// a caret to place first. Tagged with the group so parking reads this
/// group's offset and never the other's.
#[component]
fn Scroller(path: String, #[prop(into)] class: String, children: Children) -> impl IntoView {
    let state = AppState::expect();
    let scroller: NodeRef<html::Div> = NodeRef::new();
    Effect::new(move |_| {
        let Some(pending) = state.editor.viewport.get() else {
            return;
        };
        if pending.path != path {
            return;
        }
        let mine = path.clone();
        // One tick, so the content is in the document; re-read when it
        // fires, because a jump that arrived in between has cleared it.
        set_timeout(
            move || {
                let still = state.editor.viewport.get_untracked();
                if still.as_ref().is_none_or(|it| it.path != mine) {
                    return;
                }
                state.editor.viewport.set(None);
                let Some(element) = scroller.get_untracked() else {
                    return;
                };
                element.set_scroll_top(pending.top);
                element.set_scroll_left(pending.left);
                // A page's figures decode after the first layout and push
                // the text below them down. Once more when they have, if the
                // reader has not moved since — the same target, so a page
                // with no figures sees nothing happen.
                let applied = element.scroll_top();
                set_timeout(
                    move || {
                        if element.scroll_top() == applied {
                            element.set_scroll_top(pending.top);
                        }
                    },
                    std::time::Duration::from_millis(150),
                );
            },
            std::time::Duration::ZERO,
        );
    });
    view! {
        <div node_ref=scroller data-scroller=state.group.index().to_string() class=class>
            {children()}
        </div>
    }
}
