//! What a right-click offers. Local to the thing under the pointer, as a
//! context menu should be; every write in it is the same visible dock
//! command a button would run.
//!
//! A branch offers what Fork's does, less what rusty has no view for: check
//! it out (a remote one through a local branch tracking it), merge it into
//! the current branch or rebase the current branch onto it, a branch from
//! it, rename, push, delete — `-d`, which refuses unmerged work, and for a
//! remote branch a delete *on the remote*, asked first — its history alone,
//! and its name. A tag: check it out detached, a branch from it, push it,
//! delete it. A commit: its hash, a branch or a tag on it, a detached
//! checkout, cherry-pick and revert. A remote: fetch it, change its URL,
//! rename it, copy its URL, remove it — asked first.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller, format,
    state::{AppState, GitTarget, PromptKind},
    view::components::{ContextMenu, MenuItem, MenuSeparator, copy_to_clipboard},
};

/// One menu row that closes the menu after doing its thing.
fn item(
    state: AppState,
    label: String,
    danger: bool,
    run: impl Fn() + Send + Sync + 'static,
) -> AnyView {
    view! {
        <MenuItem
            label=label
            danger=danger
            on_select=Callback::new(move |_| {
                run();
                state.git.menu.set(None);
            })
        />
    }
    .into_any()
}

#[component]
pub(super) fn GitContextMenu() -> impl IntoView {
    let state = AppState::expect();
    move || {
        let menu = state.git.menu.get()?;
        let close = Callback::new(move |_| state.git.menu.set(None));
        let current = state.git.branches.with_untracked(|branches| {
            branches
                .iter()
                .find(|b| b.current)
                .map(|b| b.name.clone())
                .unwrap_or_default()
        });
        let items = match menu.target {
            GitTarget::Branch {
                name,
                remote,
                current: checked_out,
            } => branch_items(state, name, remote, checked_out, current),
            GitTarget::Tag { name, id } => {
                let (checkout, from, push, delete, copy) =
                    (name.clone(), id, name.clone(), name.clone(), name);
                view! {
                    {item(state, t!("git.checkout-tag"), false, move || controller::checkout_commit(state, checkout.clone()))}
                    {item(state, t!("git.branch-from"), false, move || {
                        controller::open_prompt(state, PromptKind::Branch { from: Some(from.clone()) })
                    })}
                    <MenuSeparator />
                    {item(state, t!("git.push-tag"), false, move || controller::push_tag(state, push.clone()))}
                    {item(state, t!("git.delete-tag"), true, move || controller::delete_tag(state, delete.clone()))}
                    <MenuSeparator />
                    {item(state, t!("git.copy-name"), false, move || copy_to_clipboard(&copy))}
                }
                .into_any()
            }
            GitTarget::Remote { name } => remote_items(state, name),
            GitTarget::Commit { id } => {
                let (copy, short, branch, tag, checkout, pick, revert) = (
                    id.clone(),
                    id.chars().take(7).collect::<String>(),
                    id.clone(),
                    id.clone(),
                    id.clone(),
                    id.clone(),
                    id,
                );
                view! {
                    {item(state, t!("git.copy-hash"), false, move || copy_to_clipboard(&copy))}
                    {item(state, t!("git.copy-short-hash"), false, move || copy_to_clipboard(&short))}
                    <MenuSeparator />
                    {item(state, t!("git.branch-here"), false, move || {
                        controller::open_prompt(state, PromptKind::Branch { from: Some(branch.clone()) })
                    })}
                    {item(state, t!("git.tag-here"), false, move || {
                        controller::open_prompt(state, PromptKind::Tag { at: tag.clone() })
                    })}
                    {item(state, t!("git.checkout-commit"), false, move || controller::checkout_commit(state, checkout.clone()))}
                    <MenuSeparator />
                    {item(state, t!("git.cherry-pick"), false, move || controller::cherry_pick(state, pick.clone()))}
                    {item(state, t!("git.revert"), false, move || controller::revert_commit(state, revert.clone()))}
                }
                .into_any()
            }
            GitTarget::Path { path } => {
                let (open, copy) = (path.clone(), path);
                view! {
                    {item(state, t!("git.open-file"), false, move || controller::open_from_git(state, open.clone()))}
                    {item(state, t!("git.copy-path"), false, move || copy_to_clipboard(&copy))}
                }
                .into_any()
            }
            // Fork's menu for a changed file, less what rusty has no view
            // for (blame, history, an external diff): open; stage or
            // unstage; discard, which asks first; stage all; stash this one
            // file; the path both ways.
            GitTarget::Change {
                path,
                staged,
                untracked,
            } => {
                let (open, copy, full, discard, stash, toggle) = (
                    path.clone(),
                    path.clone(),
                    path.clone(),
                    path.clone(),
                    path.clone(),
                    path,
                );
                let root = state
                    .project
                    .detected
                    .with_untracked(|p| p.as_ref().map(|p| p.root.clone()))
                    .unwrap_or_default();
                // The list this file is in, as the header's button sees it.
                let all: Vec<String> = state.git.status.with_untracked(|s| {
                    s.as_ref()
                        .map(|s| {
                            s.entries
                                .iter()
                                .filter(|e| {
                                    if staged {
                                        e.staged.is_some()
                                    } else {
                                        e.unstaged.is_some()
                                    }
                                })
                                .map(|e| e.path.clone())
                                .collect()
                        })
                        .unwrap_or_default()
                });
                let stage_one = if staged {
                    t!("git.unstage")
                } else {
                    t!("git.stage")
                };
                let stage_every = if staged {
                    t!("git.unstage-all")
                } else {
                    t!("git.stage-all")
                };
                let discard_label = if untracked {
                    t!("git.delete-untracked")
                } else {
                    t!("git.discard")
                };
                let nested = !staged
                    && state.git.status.with_untracked(|s| {
                        s.as_ref()
                            .is_some_and(|s| s.entries.iter().any(|e| e.path == toggle && e.nested))
                    });
                let include = toggle.clone();
                view! {
                    {nested
                        .then(|| {
                            view! {
                                {item(state, t!("git.include-nested"), false, move || {
                                    controller::include_nested(state, include.clone())
                                })}
                                <MenuSeparator />
                            }
                        })}
                    {item(state, t!("git.open-file"), false, move || controller::open_from_git(state, open.clone()))}
                    <MenuSeparator />
                    {item(state, stage_one, false, move || controller::stage(state, vec![toggle.clone()], !staged))}
                    {item(state, discard_label, true, move || controller::discard(state, discard.clone(), staged, untracked))}
                    <MenuSeparator />
                    {item(state, stage_every, false, move || controller::stage(state, all.clone(), !staged))}
                    <MenuSeparator />
                    {item(state, t!("git.stash-file"), false, move || controller::stash_file(state, stash.clone()))}
                    <MenuSeparator />
                    {item(state, t!("git.copy-path"), false, move || copy_to_clipboard(&copy))}
                    {item(state, t!("git.copy-full-path"), false, move || {
                        copy_to_clipboard(&format::full_path(&root, &full))
                    })}
                }
                .into_any()
            }
        };
        Some(view! {
            <ContextMenu x=menu.x y=menu.y on_close=close>
                {items}
            </ContextMenu>
        })
    }
}

/// A remote's menu. A group the config does not name — branches left under
/// `refs/remotes/` for a remote that is gone — has nothing to fetch from or
/// change, and offers its name alone.
fn remote_items(state: AppState, name: String) -> AnyView {
    let url = state.git.remotes.with_untracked(|remotes| {
        remotes
            .iter()
            .find(|remote| remote.name == name)
            .map(|remote| remote.url.clone())
    });
    let Some(url) = url else {
        return view! {
            {item(state, t!("git.copy-name"), false, move || copy_to_clipboard(&name))}
        }
        .into_any();
    };
    let (fetch, edit, rename, remove) = (name.clone(), name.clone(), name.clone(), name);
    view! {
        {item(state, t!("git.remote-fetch"), false, move || controller::fetch_remote(state, fetch.clone()))}
        <MenuSeparator />
        {item(state, t!("git.remote-edit-url"), false, move || {
            controller::open_prompt(state, PromptKind::RemoteUrl { name: edit.clone() })
        })}
        {item(state, t!("git.remote-rename"), false, move || {
            controller::open_prompt(state, PromptKind::RenameRemote { from: rename.clone() })
        })}
        {item(state, t!("git.remote-copy-url"), false, move || copy_to_clipboard(&url))}
        <MenuSeparator />
        {item(state, t!("git.remote-remove"), true, move || controller::remove_remote(state, remove.clone()))}
    }
    .into_any()
}

/// A branch's menu. What merge and rebase would do is named with the branch
/// checked out, since "merge into current" says nothing about which that is.
fn branch_items(
    state: AppState,
    name: String,
    remote: bool,
    checked_out: bool,
    current: String,
) -> AnyView {
    let clone = || name.clone();
    let (checkout, merge, rebase, from, rename, push, delete, filter, copy) = (
        clone(),
        clone(),
        clone(),
        clone(),
        clone(),
        clone(),
        clone(),
        clone(),
        clone(),
    );
    let can_merge = !checked_out && !current.is_empty();
    view! {
        {(!checked_out)
            .then(|| {
                let label = if remote { t!("git.checkout-remote") } else { t!("git.checkout") };
                item(state, label, false, move || controller::checkout_branch(state, checkout.clone(), remote))
            })}
        {can_merge
            .then(|| {
                let into = current.clone();
                let onto = current.clone();
                view! {
                    {item(state, t!("git.merge-into", current = into), false, move || {
                        controller::merge_into_current(state, merge.clone())
                    })}
                    {item(state, t!("git.rebase-onto", current = onto), false, move || {
                        controller::rebase_onto(state, rebase.clone())
                    })}
                }
            })}
        <MenuSeparator />
        {item(state, t!("git.branch-from"), false, move || {
            controller::open_prompt(state, PromptKind::Branch { from: Some(from.clone()) })
        })}
        {(!remote)
            .then(|| {
                view! {
                    {item(state, t!("git.rename-branch"), false, move || {
                        controller::open_prompt(state, PromptKind::Rename { from: rename.clone() })
                    })}
                    {item(state, t!("git.push-branch"), false, move || {
                        if checked_out {
                            controller::push(state)
                        } else {
                            controller::push_branch(state, push.clone())
                        }
                    })}
                }
            })}
        {(!checked_out)
            .then(|| {
                let label = if remote { t!("git.delete-remote") } else { t!("git.delete-branch") };
                item(state, label, true, move || {
                    if remote {
                        controller::delete_remote_branch(state, delete.clone())
                    } else {
                        controller::delete_branch(state, delete.clone())
                    }
                })
            })}
        <MenuSeparator />
        {item(state, t!("git.filter-branch"), false, move || controller::show_rev(state, Some(filter.clone())))}
        {item(state, t!("git.copy-name"), false, move || copy_to_clipboard(&copy))}
    }
    .into_any()
}
