//! The opened commit (or stash): message, files, one file's patch — under
//! the log or the stash list, behind a divider, with two more inside.
//!
//! Three closures with three keys. The frame is keyed on *which* commit is
//! open and whether the pane is folded; the file list's highlight on the
//! file; the patch on the file, the layout toggle and the images. Picking a
//! file used to rebuild the message and the whole list with it.
//!
//! The commit on screen stays while the next one is read, dimmed, rather
//! than the pane collapsing to a strip and growing back on every click.

use leptos::{ev, prelude::*};

use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::view::split;
use crate::{
    controller, format,
    state::{AppState, Divider, GitMenu, GitTarget},
};

use super::diff::{change_glyph, diff_pane};

#[component]
pub(super) fn Detail(#[prop(default = false)] standalone: bool) -> impl IntoView {
    let state = AppState::expect();
    let frame = Memo::new(move |_| {
        (
            state.git.selected.with(Option::is_some),
            state
                .git
                .detail
                .with(|d| d.as_ref().map(|d| d.commit.id.clone())),
            state.git.detail_hidden.get(),
        )
    });
    move || {
        let (selected, _, hidden) = frame.get();
        if !selected {
            return None;
        }
        let Some(detail) = state.git.detail.get_untracked() else {
            // The first commit of a session, not read yet.
            return Some(
                view! {
                    <div class="border-t border-line px-4 py-2 font-mono text-footnote text-label-4">
                        {move || {
                            state
                                .git
                                .selected
                                .get()
                                .map(|id| id.chars().take(7).collect::<String>())
                        }}
                    </div>
                }
                .into_any(),
            );
        };
        let commit = detail.commit.clone();
        // Folded to a strip: the hash and the summary, and the way back.
        // Fork's hide, so a wide graph can have the whole panel.
        if !standalone && hidden {
            let summary = commit.summary.clone();
            return Some(
                view! {
                    <div class="flex shrink-0 items-center gap-3 border-t border-line bg-sunken px-4 py-1.5">
                        <span class="font-mono text-footnote text-label-2">{commit.short.clone()}</span>
                        <span class="min-w-0 flex-1 truncate text-footnote text-label-3">{summary}</span>
                        <button
                            type="button"
                            title=t!("git.detail-show")
                            class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-raised hover:text-label"
                            on:click=move |_| controller::toggle_detail(state)
                        >
                            <span class="-rotate-180"><IconView icon=Icon::Chevron size=13 /></span>
                        </button>
                    </div>
                }
                .into_any(),
            );
        }
        let when = format::commit_when(commit.time);
        let exact = format::full_time(commit.time);
        let files = detail.files.clone();
        let target = state.git.selected.get_untracked().unwrap_or_default();
        // The pane's frame: in the panel, a divider above and a dragged
        // height capped at most of the panel; in a window of its own, the
        // whole window.
        let frame_class = move || {
            let base = if standalone {
                "flex min-h-0 flex-1 flex-col overflow-hidden transition-opacity"
            } else {
                "flex max-h-[80%] min-h-0 shrink-0 flex-col overflow-hidden transition-opacity"
            };
            if state.git.detail_loading.get() {
                format!("{base} opacity-60")
            } else {
                base.to_string()
            }
        };
        let height = move || {
            if standalone {
                String::new()
            } else {
                format!("height: {}px", state.layout.git_detail_height.get())
            }
        };
        let file_rows = if files.is_empty() {
            view! {
                <p class="px-3 py-1 text-footnote text-label-4">{t!("git.no-files")}</p>
            }
            .into_any()
        } else {
            files
                .iter()
                .map(|file| {
                    let path = file.path.clone();
                    let (pick, open, menu, lit) = (path.clone(), path.clone(), path.clone(), path.clone());
                    let (glyph, ink) = change_glyph(Some(file.kind), false, false);
                    let counts = match (file.added, file.removed) {
                        (Some(a), Some(r)) => format!("+{a} −{r}"),
                        _ => t!("git.binary"),
                    };
                    let class = move || {
                        if state.git.file.with(|f| f.as_deref() == Some(lit.as_str())) {
                            "flex w-full items-center gap-2 bg-selection px-3 py-0.5 text-left font-mono text-footnote"
                        } else {
                            "flex w-full items-center gap-2 px-3 py-0.5 text-left font-mono text-footnote hover:bg-sunken"
                        }
                    };
                    view! {
                        <button
                            type="button"
                            class=class
                            on:click=move |_| controller::show_commit_file(state, pick.clone())
                            on:dblclick=move |_| controller::open_from_git(state, open.clone())
                            on:contextmenu=move |event: ev::MouseEvent| {
                                event.prevent_default();
                                event.stop_propagation();
                                state.git.menu.set(Some(GitMenu {
                                    x: f64::from(event.client_x()),
                                    y: f64::from(event.client_y()),
                                    target: GitTarget::Path { path: menu.clone() },
                                }));
                            }
                        >
                            <span class=format!("w-3 shrink-0 {ink}")>{glyph}</span>
                            <span class="min-w-0 flex-1 truncate text-label-2">{path}</span>
                            <span class="shrink-0 text-label-4 tnum">{counts}</span>
                        </button>
                    }
                })
                .collect_view()
                .into_any()
        };
        let patch = move || match state.git.file.get() {
            Some(path) => {
                let text = files
                    .iter()
                    .find(|file| file.path == path)
                    .map(|file| file.patch.clone())
                    .unwrap_or_default();
                diff_pane(state, path, &text)
            }
            None => view! {
                <p class="px-4 py-3 text-footnote text-label-4">{t!("git.pick-change")}</p>
            }
            .into_any(),
        };
        Some(
            view! {
                {(!standalone).then(|| view! { <split::Handle divider=Divider::GitDetail /> })}
                // Never more than most of the panel, whatever the divider was
                // dragged to, and *everything* inside it bounded and scrolling
                // on its own: a header that took an essay-length message's
                // natural height once pushed this pane straight over the dock.
                <div class=frame_class style=height>
                    // The message block scrolls *itself*: it is the element
                    // the cap is on. A child scrolling inside a flex row took
                    // its content height, and an essay-length message painted
                    // straight over the files and the patch below.
                    <div
                        class="shrink-0 overflow-y-auto bg-sunken px-4 py-2"
                        style=move || format!("max-height: {}px", state.layout.git_message_height.get())
                    >
                        <div class="flex items-center gap-2 text-footnote text-label-3">
                            <span class="font-mono text-label-2 select-text">{commit.short.clone()}</span>
                            <span>{commit.author.clone()}</span>
                            <span class="text-label-4" title=exact>{when}</span>
                            <span class="flex-1" />
                            {(!standalone).then(|| view! {
                                <div class="flex shrink-0 items-center gap-0.5">
                                    <button
                                        type="button"
                                        title=t!("git.detail-window")
                                        class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-raised hover:text-label"
                                        on:click=move |_| controller::open_commit_window(state, target.clone())
                                    >
                                        <IconView icon=Icon::External size=13 />
                                    </button>
                                    <button
                                        type="button"
                                        title=t!("git.detail-hide")
                                        class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-raised hover:text-label"
                                        on:click=move |_| controller::toggle_detail(state)
                                    >
                                        <IconView icon=Icon::Close size=13 />
                                    </button>
                                </div>
                            })}
                        </div>
                        <p class="mt-1 text-body whitespace-pre-wrap select-text">{detail.body.clone()}</p>
                    </div>
                    <split::Handle divider=Divider::GitMessage />
                    <div class="flex min-h-0 flex-1">
                        <div
                            class="shrink-0 overflow-y-auto bg-sidebar py-1"
                            style=move || format!("width: {}px", state.layout.git_files_width.get())
                        >
                            {file_rows}
                        </div>
                        <split::Handle divider=Divider::GitFiles />
                        {patch}
                    </div>
                </div>
            }
            .into_any(),
        )
    }
}
