//! The working tree: staged and not, one file's diff, and the commit box.
//!
//! Two columns, as Fork and VS Code both lay it out: the files and the
//! commit box on the left, the diff on the right at full height. Stacked,
//! the diff and the commit box took the height between them and the file
//! lists — the thing this view is for — were left three rows tall in a
//! panel two thousand pixels wide.
//!
//! The lists are keyed on the status alone; which row is lit is each row's
//! own class, so clicking a file draws its diff and nothing else.

use leptos::{ev, prelude::*};

use rusty_git::StatusEntry;
use rusty_i18n::t;

use crate::view::split;
use crate::{
    controller,
    state::{AppState, Divider, GitMenu, GitTarget},
    view::components::{Button, ButtonKind},
};

use super::diff::{change_glyph, diff_pane};

#[component]
pub(super) fn Changes() -> impl IntoView {
    let state = AppState::expect();
    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            {move || {
                let Some(status) = state.git.status.get() else {
                    return view! {
                        <p class="px-4 py-3 text-callout text-label-3">{t!("git.loading")}</p>
                    }
                    .into_any();
                };
                let head = if status.detached {
                    t!("git.detached")
                } else {
                    status.head.clone().unwrap_or_default()
                };
                let tracking = match &status.upstream {
                    Some(upstream) => t!(
                        "git.ahead-behind",
                        ahead = status.ahead,
                        behind = status.behind,
                        upstream = upstream.clone()
                    ),
                    None => t!("git.no-upstream"),
                };
                view! {
                    <div class="flex items-center gap-2 border-b border-line px-4 py-1.5 text-footnote text-label-3">
                        <span class="font-mono text-label-2">{head}</span>
                        <span class="text-label-4">{tracking}</span>
                    </div>
                }
                .into_any()
            }}
            <div class="flex min-h-0 flex-1">
                <div
                    class="flex min-h-0 shrink-0 flex-col bg-sidebar"
                    style=move || format!("width: {}px", state.layout.git_changes_width.get())
                >
                    <div class="min-h-0 flex-1 overflow-y-auto py-1">
                        {move || {
                            let Some(status) = state.git.status.get() else {
                                return ().into_any();
                            };
                            if status.entries.is_empty() {
                                return view! {
                                    <p class="px-4 py-3 text-callout text-label-3">{t!("git.clean")}</p>
                                }
                                .into_any();
                            }
                            let staged: Vec<StatusEntry> = status
                                .entries
                                .iter()
                                .filter(|e| e.staged.is_some())
                                .cloned()
                                .collect();
                            let unstaged: Vec<StatusEntry> = status
                                .entries
                                .iter()
                                .filter(|e| e.unstaged.is_some())
                                .cloned()
                                .collect();
                            view! {
                                {change_list(state, t!("git.staged"), staged, true)}
                                {change_list(state, t!("git.unstaged"), unstaged, false)}
                            }
                            .into_any()
                        }}
                    </div>
                    <CommitBox />
                </div>
                <split::Handle divider=Divider::GitChanges />
                <div class="flex min-h-0 min-w-0 flex-1">
                    {move || {
                        let path = state.git.diff_for.get().map(|(path, _)| path);
                        match (path, state.git.diff.get()) {
                            (Some(path), Some(text)) => diff_pane(state, path, &text),
                            _ => view! {
                                <p class="px-4 py-3 text-footnote text-label-4">{t!("git.pick-change")}</p>
                            }
                                .into_any(),
                        }
                    }}
                </div>
            </div>
        </div>
    }
}

/// One side of the working tree: a heading with an all-or-nothing action,
/// then a row per file with the other action beside it.
fn change_list(state: AppState, title: String, entries: Vec<StatusEntry>, staged: bool) -> AnyView {
    if entries.is_empty() {
        return ().into_any();
    }
    let all: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
    let count = entries.len();
    let all_label = if staged {
        t!("git.unstage-all")
    } else {
        t!("git.stage-all")
    };
    let row_title = if staged {
        t!("git.unstage")
    } else {
        t!("git.stage")
    };
    view! {
        <div class="flex items-center gap-2 px-4 pt-2 pb-1">
            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">{title}</span>
            <span class="text-caption text-label-4 tnum">{count}</span>
            <span class="flex-1" />
            <button
                type="button"
                class="text-footnote text-label-3 hover:text-label"
                on:click=move |_| controller::stage(state, all.clone(), !staged)
            >
                {all_label}
            </button>
        </div>
        {entries
            .into_iter()
            .map(|entry| {
                let kind = if staged { entry.staged } else { entry.unstaged };
                let (glyph, ink) = change_glyph(kind, entry.untracked && !staged, entry.conflicted);
                // The two glyphs that are not a letter get a word on hover.
                let hint = if entry.conflicted {
                    Some(t!("git.conflicted"))
                } else if entry.untracked && !staged {
                    Some(t!("git.untracked"))
                } else {
                    None
                };
                let path = entry.path.clone();
                let (show, open, menu, toggle, lit) =
                    (path.clone(), path.clone(), path.clone(), path.clone(), path.clone());
                let untracked = entry.untracked;
                let class = move || {
                    let on = state
                        .git
                        .diff_for
                        .with(|chosen| chosen.as_ref().is_some_and(|(p, s)| *p == lit && *s == staged));
                    if on {
                        "group flex w-full items-center gap-2 bg-selection px-4 py-0.5 text-left font-mono text-footnote"
                    } else {
                        "group flex w-full items-center gap-2 px-4 py-0.5 text-left font-mono text-footnote hover:bg-sunken"
                    }
                };
                let title = row_title.clone();
                view! {
                    <div
                        class=class
                        on:contextmenu=move |event: ev::MouseEvent| {
                            event.prevent_default();
                            event.stop_propagation();
                            state.git.menu.set(Some(GitMenu {
                                x: f64::from(event.client_x()),
                                y: f64::from(event.client_y()),
                                target: GitTarget::Change {
                                    path: menu.clone(),
                                    staged,
                                    untracked: untracked && !staged,
                                },
                            }));
                        }
                    >
                        <span class=format!("w-3 shrink-0 {ink}") title=hint>{glyph}</span>
                        <button
                            type="button"
                            class="min-w-0 flex-1 truncate text-left text-label-2"
                            on:click=move |_| controller::load_diff(state, show.clone(), staged, untracked)
                            on:dblclick=move |_| controller::open_from_git(state, open.clone())
                        >
                            {path}
                        </button>
                        <button
                            type="button"
                            title=title
                            class="shrink-0 rounded-[4px] px-1.5 text-label-4 ring-1 ring-line hover:bg-raised hover:text-label"
                            on:click=move |_| controller::stage(state, vec![toggle.clone()], !staged)
                        >
                            {if staged { "−" } else { "+" }}
                        </button>
                    </div>
                }
            })
            .collect_view()}
    }
    .into_any()
}

/// The message and the button. Ctrl+Enter commits, as every git client's
/// message box does; a checkbox turns the commit into an amend.
#[component]
fn CommitBox() -> impl IntoView {
    let state = AppState::expect();
    let staged_count = Signal::derive(move || {
        state.git.status.with(|s| {
            s.as_ref()
                .map(|s| s.entries.iter().filter(|e| e.staged.is_some()).count())
                .unwrap_or(0)
        })
    });
    // An amend may go without a message (it keeps the one it has) and
    // without anything staged (it only rewords); a commit may do neither.
    let blocked = Signal::derive(move || {
        let no_author = state
            .git
            .identity
            .with(|id| id.as_ref().is_some_and(|id| !id.complete()));
        no_author
            || (!state.git.amend.get()
                && (staged_count.get() == 0 || state.git.message.with(|m| m.trim().is_empty())))
    });
    view! {
        <div class="shrink-0 border-t border-line bg-sunken px-4 py-3">
            // Git's identity, asked for here rather than discovered as
            // "Author identity unknown" in the dock after the button. Shown
            // only once git has said which of the two is missing, prefilled
            // with whatever is set, and saved the way git's own hint says:
            // `git config --global`, or for this repository alone.
            {move || {
                let identity = state.git.identity.get()?;
                if identity.complete() {
                    return None;
                }
                let name = RwSignal::new(identity.name.clone().unwrap_or_default());
                let email = RwSignal::new(identity.email.clone().unwrap_or_default());
                let local = RwSignal::new(false);
                let ready = Signal::derive(move || {
                    !name.with(|n| n.trim().is_empty()) && !email.with(|e| e.trim().is_empty())
                });
                Some(view! {
                    <div class="mb-3 rounded-[8px] bg-amber-fill px-3 py-2.5">
                        <p class="text-callout font-medium">{t!("git.identity-title")}</p>
                        <p class="mt-0.5 text-footnote text-label-2">{t!("git.identity-detail")}</p>
                        <div class="mt-2 flex flex-wrap items-center gap-2">
                            <input
                                type="text"
                                placeholder=t!("git.identity-name")
                                class="h-[28px] min-w-[10rem] flex-1 rounded-[6px] bg-sunken px-2.5 text-footnote outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                                prop:value=move || name.get()
                                on:input=move |event| name.set(event_target_value(&event))
                            />
                            <input
                                type="email"
                                placeholder=t!("git.identity-email")
                                class="h-[28px] min-w-[12rem] flex-1 rounded-[6px] bg-sunken px-2.5 text-footnote outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                                prop:value=move || email.get()
                                on:input=move |event| email.set(event_target_value(&event))
                            />
                        </div>
                        <div class="mt-2 flex items-center gap-3">
                            <Button
                                label=t!("git.identity-save")
                                kind=ButtonKind::Primary
                                disabled=Signal::derive(move || !ready.get())
                                on_click=Callback::new(move |_| {
                                    controller::set_identity(
                                        state,
                                        name.get_untracked(),
                                        email.get_untracked(),
                                        local.get_untracked(),
                                    )
                                })
                            />
                            <label class="flex items-center gap-1.5 text-footnote text-label-3 select-none">
                                <input
                                    type="checkbox"
                                    prop:checked=move || local.get()
                                    on:change=move |event| local.set(event_target_checked(&event))
                                />
                                {t!("git.identity-local")}
                            </label>
                        </div>
                    </div>
                })
            }}
            <textarea
                rows="3"
                placeholder=t!("git.commit-placeholder")
                class="w-full resize-none rounded-[8px] bg-sunken px-3 py-2 text-body outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                prop:value=move || state.git.message.get()
                on:input=move |event| state.git.message.set(event_target_value(&event))
                on:keydown=move |event: ev::KeyboardEvent| {
                    if event.key() == "Enter" && event.ctrl_key() && !blocked.get_untracked() {
                        event.prevent_default();
                        controller::commit(state);
                    }
                }
            />
            <div class="mt-2 flex items-center gap-3">
                {move || {
                    let label = if state.git.amend.get() {
                        t!("git.amend-button")
                    } else {
                        t!("git.commit")
                    };
                    view! {
                        <Button
                            label=label
                            kind=ButtonKind::Primary
                            disabled=blocked
                            on_click=Callback::new(move |_| controller::commit(state))
                        />
                    }
                }}
                <label class="flex items-center gap-1.5 text-footnote text-label-3 select-none">
                    <input
                        type="checkbox"
                        prop:checked=move || state.git.amend.get()
                        on:change=move |event| controller::amend_toggle(state, event_target_checked(&event))
                    />
                    {t!("git.amend")}
                </label>
            </div>
        </div>
    }
}
