//! Branches, remotes and tags, down the side — Fork's left column.
//!
//! It was a picker: a menu of branch names whose rows *filtered the log*,
//! with checkout and delete appearing only after a filter was chosen. Seeing
//! a branch and switching to it were one gesture and doing anything else to
//! it took two. Now every ref is a row: a click goes to its commit in the
//! log, a double-click checks a branch out, the funnel on hover shows that
//! branch's history alone, and a right-click offers the rest. A local branch
//! says how far it is ahead of and behind its upstream, and that the
//! upstream is gone when the remote has deleted it.
//!
//! Remote branches are grouped by remote, and a remote branch checked out
//! becomes a local branch tracking it (`controller::checkout_args`) rather
//! than a detached HEAD, which is never what a double-click meant.

use std::collections::BTreeMap;

use leptos::{ev, prelude::*};

use rusty_git::{Branch, Tag};
use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::{
    controller, format,
    state::{AppState, GitMenu, GitTarget},
};

#[component]
pub(super) fn Sidebar() -> impl IntoView {
    let state = AppState::expect();
    view! {
        <div
            class="flex min-h-0 shrink-0 flex-col border-r border-line bg-sidebar"
            style=move || format!("width: {}px", state.layout.git_sidebar_width.get())
        >
            <div class="px-2 pt-2 pb-1">
                <input
                    type="text"
                    spellcheck="false"
                    placeholder=t!("git.filter-refs")
                    class="h-[26px] w-full rounded-[6px] bg-sunken px-2 text-footnote outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                    prop:value=move || state.git.ref_filter.get()
                    on:input=move |event| state.git.ref_filter.set(event_target_value(&event))
                    on:keydown=move |event: ev::KeyboardEvent| {
                        if event.key() == "Escape" {
                            state.git.ref_filter.set(String::new());
                        }
                    }
                />
            </div>
            <div class="min-h-0 flex-1 overflow-y-auto pb-2">
                {move || {
                    let filter = state.git.ref_filter.get().trim().to_lowercase();
                    let keep = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
                    let (local, remote): (Vec<Branch>, Vec<Branch>) = state
                        .git
                        .branches
                        .get()
                        .into_iter()
                        .filter(|b| keep(&b.name))
                        .partition(|b| !b.remote);
                    let mut remotes: BTreeMap<String, Vec<Branch>> = BTreeMap::new();
                    for branch in remote {
                        let group = branch.remote_name.clone().unwrap_or_default();
                        remotes.entry(group).or_default().push(branch);
                    }
                    let tags: Vec<Tag> = state
                        .git
                        .tags
                        .get()
                        .into_iter()
                        .filter(|t| keep(&t.name))
                        .collect();
                    let local_count = local.len();
                    let local_rows = local
                        .into_iter()
                        .map(|branch| branch_row(state, branch, 0))
                        .collect_view()
                        .into_any();
                    let remote_sections = remotes
                        .into_iter()
                        .map(|(remote, branches)| {
                            let count = branches.len();
                            let rows = branches
                                .into_iter()
                                .map(|branch| branch_row(state, branch, 1))
                                .collect_view()
                                .into_any();
                            section(state, format!("remote:{remote}"), remote, count, rows)
                        })
                        .collect_view();
                    let tag_count = tags.len();
                    let tag_rows = tags
                        .into_iter()
                        .map(|tag| tag_row(state, tag))
                        .collect_view()
                        .into_any();
                    view! {
                        {section(state, "local".to_string(), t!("git.local"), local_count, local_rows)}
                        {remote_sections}
                        {section(state, "tags".to_string(), t!("git.tags"), tag_count, tag_rows)}
                    }
                }}
            </div>
        </div>
    }
}

/// A foldable heading and its rows. Folded sections are remembered for the
/// session: a remote with two hundred branches is usually folded for good.
fn section(state: AppState, key: String, title: String, count: usize, rows: AnyView) -> AnyView {
    let folded = {
        let key = key.clone();
        Signal::derive(move || state.git.folded.with(|folded| folded.contains(&key)))
    };
    let toggle = move |_| {
        state
            .git
            .folded
            .update(|folded| match folded.iter().position(|k| *k == key) {
                Some(at) => {
                    folded.remove(at);
                }
                None => folded.push(key.clone()),
            })
    };
    view! {
        <button
            type="button"
            on:click=toggle
            class="flex w-full items-center gap-1 px-2 pt-2 pb-0.5 text-left text-caption font-semibold tracking-[0.06em] text-label-3 uppercase hover:text-label"
        >
            <span class=move || if folded.get() { "-rotate-90 transition-transform" } else { "transition-transform" }>
                <IconView icon=Icon::Chevron size=10 />
            </span>
            <span class="min-w-0 flex-1 truncate normal-case tracking-normal">{title}</span>
            <span class="text-label-4 tnum">{count}</span>
        </button>
        <div class:hidden=move || folded.get()>{rows}</div>
    }
    .into_any()
}

/// The classes a ref row wears, lit when its commit is the one open.
fn row_class(lit: bool) -> &'static str {
    if lit {
        "group flex w-full cursor-pointer items-center gap-1.5 bg-selection py-[3px] pr-2 text-left"
    } else {
        "group flex w-full cursor-pointer items-center gap-1.5 py-[3px] pr-2 text-left hover:bg-sunken"
    }
}

fn branch_row(state: AppState, branch: Branch, depth: u32) -> AnyView {
    let Branch {
        name,
        current,
        remote,
        id,
        ahead,
        behind,
        gone,
        time,
        subject,
        upstream,
        ..
    } = branch.clone();
    let shown = branch.local_name().to_string();
    let lit = {
        let id = id.clone();
        Signal::derive(move || {
            state
                .git
                .selected
                .with(|s| s.as_deref() == Some(id.as_str()))
        })
    };
    let tip = format!(
        "{name}\n{subject}\n{}{}",
        format::full_time(time),
        upstream
            .as_deref()
            .map(|u| format!("\n→ {u}"))
            .unwrap_or_default()
    );
    let (go, open, filter, menu) = (
        (id.clone(), name.clone()),
        name.clone(),
        name.clone(),
        name.clone(),
    );
    view! {
        <div
            class=move || row_class(lit.get())
            style=format!("padding-left: {}px", 10 + depth * 12)
            title=tip
            on:click=move |_| controller::reveal_ref(state, go.0.clone(), Some(go.1.clone()))
            on:dblclick=move |_| {
                if !current {
                    controller::checkout_branch(state, open.clone(), remote);
                }
            }
            on:contextmenu=move |event: ev::MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
                state.git.menu.set(Some(GitMenu {
                    x: f64::from(event.client_x()),
                    y: f64::from(event.client_y()),
                    target: GitTarget::Branch {
                        name: menu.clone(),
                        remote,
                        current,
                    },
                }));
            }
        >
            <span class="w-2.5 shrink-0 text-center text-caption text-patina">
                {if current { "●" } else { "" }}
            </span>
            <span class=if current {
                "min-w-0 flex-1 truncate font-mono text-footnote font-semibold text-label"
            } else {
                "min-w-0 flex-1 truncate font-mono text-footnote text-label-2"
            }>{shown}</span>
            {gone.then(|| view! { <span class="shrink-0 text-caption text-crimson">{t!("git.gone")}</span> })}
            {(ahead > 0)
                .then(|| view! { <span class="shrink-0 text-caption text-label-3 tnum">{format!("↑{ahead}")}</span> })}
            {(behind > 0)
                .then(|| view! { <span class="shrink-0 text-caption text-label-3 tnum">{format!("↓{behind}")}</span> })}
            <button
                type="button"
                title=t!("git.filter-branch")
                class="invisible grid size-5 shrink-0 place-items-center rounded-[4px] text-label-3 group-hover:visible hover:bg-raised hover:text-label"
                on:click=move |event: ev::MouseEvent| {
                    event.stop_propagation();
                    controller::show_rev(state, Some(filter.clone()));
                }
            >
                <IconView icon=Icon::Filter size=11 />
            </button>
        </div>
    }
    .into_any()
}

fn tag_row(state: AppState, tag: Tag) -> AnyView {
    let Tag {
        name,
        id,
        time,
        subject,
    } = tag;
    let lit = {
        let id = id.clone();
        Signal::derive(move || {
            state
                .git
                .selected
                .with(|s| s.as_deref() == Some(id.as_str()))
        })
    };
    let tip = format!("{name}\n{subject}\n{}", format::full_time(time));
    let (go, menu, menu_id) = ((id.clone(), name.clone()), name.clone(), id);
    view! {
        <div
            class=move || row_class(lit.get())
            style="padding-left: 10px"
            title=tip
            on:click=move |_| controller::reveal_ref(state, go.0.clone(), Some(go.1.clone()))
            on:contextmenu=move |event: ev::MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
                state.git.menu.set(Some(GitMenu {
                    x: f64::from(event.client_x()),
                    y: f64::from(event.client_y()),
                    target: GitTarget::Tag {
                        name: menu.clone(),
                        id: menu_id.clone(),
                    },
                }));
            }
        >
            <span class="w-2.5 shrink-0" />
            <span class="min-w-0 flex-1 truncate font-mono text-footnote text-label-2">{name}</span>
            <span class="shrink-0 text-caption text-label-4 tnum">{format::commit_when(time)}</span>
        </div>
    }
    .into_any()
}
