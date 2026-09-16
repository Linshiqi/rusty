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
//!
//! The remotes are listed from the config, not inferred from their branches.
//! Inferred, a remote nothing had been fetched from did not exist on screen,
//! and the section it would have been in did not exist either — so there was
//! nowhere to add one, and a repository made with `git init` could not be
//! connected to GitHub from the panel at all. The section is always drawn
//! now, with `+` on its heading and a right-click on each remote.

use std::collections::BTreeMap;

use leptos::{ev, prelude::*};

use rusty_git::{Branch, Remote, Tag};
use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::{
    controller, format,
    state::{AppState, GitMenu, GitTarget, PromptKind},
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
                        .partition(|b| !b.remote);
                    let local: Vec<Branch> = local.into_iter().filter(|b| keep(&b.name)).collect();
                    // Every remote the config names, and any a branch names
                    // that the config does not — refs left behind by hand —
                    // so nothing under `refs/remotes/` goes unshown.
                    let mut groups: BTreeMap<String, (Option<Remote>, Vec<Branch>)> = BTreeMap::new();
                    for configured in state.git.remotes.get() {
                        let name = configured.name.clone();
                        groups.entry(name).or_default().0 = Some(configured);
                    }
                    for branch in remote {
                        let group = branch.remote_name.clone().unwrap_or_default();
                        groups.entry(group).or_default().1.push(branch);
                    }
                    let remote_count = groups.len();
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
                    let searching = !filter.is_empty();
                    let remote_groups = groups
                        .into_iter()
                        .filter_map(|(name, (configured, branches))| {
                            // A remote whose name matches shows all of its
                            // branches; otherwise only the ones that match,
                            // and the remote only if any do.
                            let named = keep(&name);
                            let branches: Vec<Branch> = if named {
                                branches
                            } else {
                                branches.into_iter().filter(|b| keep(&b.name)).collect()
                            };
                            (!searching || named || !branches.is_empty())
                                .then(|| remote_group(state, name, configured, branches))
                        })
                        .collect_view();
                    let remote_rows = view! {
                        {remote_groups}
                        {(remote_count == 0 && !searching)
                            .then(|| {
                                view! {
                                    <button
                                        type="button"
                                        class="flex w-full items-center gap-1.5 py-[3px] pr-2 pl-[22px] text-left text-footnote text-label-3 hover:bg-sunken hover:text-label"
                                        on:click=move |_| controller::open_prompt(state, PromptKind::Remote { push: false })
                                    >
                                        <IconView icon=Icon::Plus size=10 />
                                        {t!("git.add-remote")}
                                    </button>
                                }
                            })}
                    }
                    .into_any();
                    let add_remote = view! {
                        <button
                            type="button"
                            title=t!("git.add-remote")
                            class="mt-1.5 grid size-5 shrink-0 place-items-center rounded-[4px] text-label-3 hover:bg-raised hover:text-label"
                            on:click=move |_| controller::open_prompt(state, PromptKind::Remote { push: false })
                        >
                            <IconView icon=Icon::Plus size=11 />
                        </button>
                    }
                    .into_any();
                    let tag_count = tags.len();
                    let tag_rows = tags
                        .into_iter()
                        .map(|tag| tag_row(state, tag))
                        .collect_view()
                        .into_any();
                    view! {
                        {section(state, "local".to_string(), t!("git.local"), local_count, local_rows, None)}
                        {section(state, "remotes".to_string(), t!("git.remotes"), remote_count, remote_rows, Some(add_remote))}
                        {section(state, "tags".to_string(), t!("git.tags"), tag_count, tag_rows, None)}
                    }
                }}
            </div>
        </div>
    }
}

/// A foldable heading and its rows, with an action beside the heading when
/// the section has one. Folded sections are remembered for the session.
fn section(
    state: AppState,
    key: String,
    title: String,
    count: usize,
    rows: AnyView,
    action: Option<AnyView>,
) -> AnyView {
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
        // The heading is a row holding a button rather than a button: the
        // action beside it is a button too, and one inside another is not
        // something a browser will draw as two.
        <div class="flex w-full items-center pr-1">
            <button
                type="button"
                on:click=toggle
                class="flex min-w-0 flex-1 items-center gap-1 px-2 pt-2 pb-0.5 text-left text-caption font-semibold tracking-[0.06em] text-label-3 uppercase hover:text-label"
            >
                <span class=move || if folded.get() { "-rotate-90 transition-transform" } else { "transition-transform" }>
                    <IconView icon=Icon::Chevron size=10 />
                </span>
                <span class="min-w-0 flex-1 truncate normal-case tracking-normal">{title}</span>
                <span class="text-label-4 tnum">{count}</span>
            </button>
            {action}
        </div>
        <div class:hidden=move || folded.get()>{rows}</div>
    }
    .into_any()
}

/// One remote: its name, folding its branches, with the URL in the tooltip
/// and its verbs on a right-click. A remote nothing has been fetched from
/// says so, since an empty fold reads as a remote that is broken.
fn remote_group(
    state: AppState,
    name: String,
    configured: Option<Remote>,
    branches: Vec<Branch>,
) -> AnyView {
    let key = format!("remote:{name}");
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
    let tip = match &configured {
        Some(Remote {
            url,
            push_url: Some(push),
            ..
        }) => t!("git.remote-urls", fetch = url.clone(), push = push.clone()),
        Some(remote) => remote.url.clone(),
        None => t!("git.remote-unconfigured"),
    };
    let count = branches.len();
    let empty = branches.is_empty() && configured.is_some();
    let rows = branches
        .into_iter()
        .map(|branch| branch_row(state, branch, 1))
        .collect_view();
    let menu = name.clone();
    view! {
        <div
            class="flex w-full cursor-pointer items-center gap-1 py-[3px] pr-2 pl-[10px] text-left hover:bg-sunken"
            title=tip
            on:click=toggle
            on:contextmenu=move |event: ev::MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
                state.git.menu.set(Some(GitMenu {
                    x: f64::from(event.client_x()),
                    y: f64::from(event.client_y()),
                    target: GitTarget::Remote { name: menu.clone() },
                }));
            }
        >
            <span class=move || if folded.get() { "-rotate-90 text-label-3 transition-transform" } else { "text-label-3 transition-transform" }>
                <IconView icon=Icon::Chevron size=10 />
            </span>
            <span class="min-w-0 flex-1 truncate font-mono text-footnote text-label-2">{name}</span>
            <span class="shrink-0 text-caption text-label-4 tnum">{count}</span>
        </div>
        <div class:hidden=move || folded.get()>
            {rows}
            {empty
                .then(|| {
                    view! {
                        <div class="py-[3px] pr-2 pl-[34px] text-caption text-label-4">
                            {t!("git.remote-empty")}
                        </div>
                    }
                })}
        </div>
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
