//! The repository: its history, its working tree, its branches and stashes,
//! and the commands that move any of them.
//!
//! Two kinds of call, on purpose. **Reads** — the log, a commit, the status,
//! the stash list, one path's diff — are IPC calls that answer with model
//! types and never touch the dock. **Writes** run as visible dock commands
//! through the same runner every `cargo` and `espflash` uses, so the exact
//! `git` line and everything git says back are readable, and a failure on a
//! dirty tree or a rejected push is a paragraph in the dock rather than a
//! banner nobody can act on. The one exception is staging: `git add` on a
//! file is instant and reversible, and a dock line per click would bury the
//! commands that matter under the ones that do not.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_git::{Branch, CommitDetail, History, Stash, Status};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, remember_split},
};

/// The log for the chosen branch, or for every branch.
///
/// Not through `track`: a project that is not a repository is an ordinary
/// thing to open, and "not inside a git repository" belongs in the panel
/// that asked, not on the banner over the whole window.
pub fn load_history(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        rev: Option<String>,
        limit: usize,
    }
    let args = Args {
        rev: state.git.rev.get_untracked(),
        limit: state.git.limit.get_untracked(),
    };
    spawn_local(async move {
        match ipc::call::<_, History>(cmd::git::HISTORY, &args).await {
            Ok(history) => {
                state.git.history.set(Some(history));
                state.git.unavailable.set(None);
            }
            Err(error) => {
                state.git.history.set(None);
                state.git.unavailable.set(Some(error.message));
            }
        }
        state.git.loaded.set(true);
    });
}

/// The branches, for the strip above the log. Quiet on failure: the history
/// call has already said why, once.
pub fn load_branches(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    spawn_local(async move {
        if let Ok(branches) = ipc::call::<_, Vec<Branch>>(cmd::git::BRANCHES, &()).await {
            state.git.branches.set(branches);
        }
    });
}

/// Where the working tree stands. Quiet on failure, like the branches.
pub fn load_status(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    spawn_local(async move {
        if let Ok(status) = ipc::call::<_, Status>(cmd::git::STATUS, &()).await {
            state.git.status.set(Some(status));
        }
    });
}

pub fn load_stashes(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    spawn_local(async move {
        if let Ok(stashes) = ipc::call::<_, Vec<Stash>>(cmd::git::STASHES, &()).await {
            state.git.stashes.set(stashes);
        }
    });
}

/// Everything again. What the watcher calls after every batch and the
/// refresh button calls on demand — a no-op until the panel has asked once,
/// so a project nobody has looked at the history of costs no `git log` per
/// save.
pub fn refresh_git(state: AppState) {
    if !state.git.loaded.get_untracked() {
        return;
    }
    load_history(state);
    load_branches(state);
    load_status(state);
    load_stashes(state);
}

/// Every read at once — the panel opening, or a project changing under it.
pub fn load_git(state: AppState) {
    load_history(state);
    load_branches(state);
    load_status(state);
    load_stashes(state);
}

/// Show one branch's history, or every branch's when `rev` is `None`.
pub fn show_rev(state: AppState, rev: Option<String>) {
    state.git.rev.set(rev);
    load_history(state);
}

/// Ask for older commits: twice as many as now. The backend caps it.
pub fn show_more(state: AppState) {
    let limit = state.git.limit.get_untracked();
    state.git.limit.set((limit * 2).min(10_000));
    load_history(state);
}

/// Side by side or one column, for every diff the panel shows — and
/// remembered, so the choice outlives the window.
pub fn set_split(state: AppState, on: bool) {
    state.git.split.set(on);
    remember_split(on);
}

/// Open a commit: its message, its files, their patches.
pub fn select_commit(state: AppState, id: String) {
    #[derive(serde::Serialize)]
    struct Args {
        id: String,
    }
    state.git.selected.set(Some(id.clone()));
    state.git.detail.set(None);
    state.git.file.set(None);
    let args = Args { id: id.clone() };
    track(
        state,
        async move { ipc::call::<_, CommitDetail>(cmd::git::COMMIT, &args).await },
        move |detail| {
            // A later click wins: the answer to an earlier one arriving after
            // it must not replace what the user is looking at now.
            if state.git.selected.get_untracked().as_deref() != Some(id.as_str()) {
                return;
            }
            state
                .git
                .file
                .set(detail.files.first().map(|f| f.path.clone()));
            state.git.detail.set(Some(detail));
        },
    );
}

/// One working-tree path's diff, for the Changes view.
pub fn load_diff(state: AppState, path: String, staged: bool, untracked: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        staged: bool,
        untracked: bool,
    }
    let key = (path.clone(), staged);
    state.git.diff_for.set(Some(key.clone()));
    state.git.diff.set(None);
    let args = Args {
        path,
        staged,
        untracked,
    };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::git::DIFF, &args).await },
        move |text| {
            if state.git.diff_for.get_untracked().as_ref() == Some(&key) {
                state.git.diff.set(Some(text));
            }
        },
    );
}

/// Put paths in the index, or take them out. Quiet — see the module header —
/// and followed by a status read, since the answer is the new status.
pub fn stage(state: AppState, paths: Vec<String>, on: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        paths: Vec<String>,
        on: bool,
    }
    let args = Args { paths, on };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::git::STAGE, &args).await },
        move |()| {
            load_status(state);
            // The diff showing is of a side that may have just moved.
            if let Some((path, staged)) = state.git.diff_for.get_untracked() {
                let untracked = state.git.status.with_untracked(|s| {
                    s.as_ref()
                        .and_then(|s| s.entries.iter().find(|e| e.path == path))
                        .is_some_and(|e| e.untracked)
                });
                load_diff(state, path, staged, untracked);
            }
        },
    );
}

/// After any command that changed the repository: read everything back, and
/// let the tree and the open files follow the disk.
fn after_git(state: AppState) {
    refresh_git(state);
    refresh_tree(state);
}

/// Commit what is staged, with the message being written — or amend the
/// last commit with it. The message is one argument however many lines it
/// has: through the argument-vector runner, never the line splitter.
///
/// Only an amend may go without a message; it then keeps the one it has
/// (`--no-edit`) rather than opening an editor nobody can see.
pub fn commit(state: AppState) {
    let message = state.git.message.get_untracked();
    let amend = state.git.amend.get_untracked();
    let mut args = vec!["commit".to_string()];
    if amend {
        args.push("--amend".to_string());
    }
    if message.trim().is_empty() {
        if !amend {
            return;
        }
        args.push("--no-edit".to_string());
    } else {
        args.push("-m".to_string());
        args.push(message);
    }
    run_args_at_root_then(state, "git", args, move |code| {
        if code == Some(0) {
            state.git.message.set(String::new());
            state.git.amend.set(false);
        }
        after_git(state);
    });
}

/// Turn amending on or off. Turning it on over an empty box fills the box
/// with HEAD's whole message — the one the amend replaces — because `-m`
/// with only a summary typed would silently cut an essay down to its first
/// line. A message already being written is left alone.
pub fn amend_toggle(state: AppState, on: bool) {
    state.git.amend.set(on);
    if !on || !state.git.message.with_untracked(|m| m.trim().is_empty()) {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        id: String,
    }
    let args = Args { id: "HEAD".into() };
    track(
        state,
        async move { ipc::call::<_, CommitDetail>(cmd::git::COMMIT, &args).await },
        move |detail| {
            // Still amending, and nothing typed meanwhile.
            if state.git.amend.get_untracked()
                && state.git.message.with_untracked(|m| m.trim().is_empty())
            {
                state.git.message.set(detail.body);
            }
        },
    );
}

/// Check a commit out by hash — a detached HEAD, said so in the dock.
pub fn checkout_commit(state: AppState, id: String) {
    run_args_at_root_then(
        state,
        "git",
        vec!["checkout".to_string(), "--detach".to_string(), id],
        move |_| after_git(state),
    );
}

/// Apply one commit's change on top of the current branch.
pub fn cherry_pick(state: AppState, id: String) {
    run_args_at_root_then(
        state,
        "git",
        vec!["cherry-pick".to_string(), id],
        move |_| after_git(state),
    );
}

/// A new commit undoing an old one. `--no-edit` takes git's own message —
/// an editor would open on a terminal nobody is watching.
pub fn revert_commit(state: AppState, id: String) {
    run_args_at_root_then(
        state,
        "git",
        vec!["revert".to_string(), "--no-edit".to_string(), id],
        move |_| after_git(state),
    );
}

/// Stash the working tree, untracked files included — "everything I have"
/// is what the button says — with the note if one was written.
pub fn stash_save(state: AppState) {
    let note = state.git.stash_note.get_untracked();
    let mut args = vec![
        "stash".to_string(),
        "push".to_string(),
        "--include-untracked".to_string(),
    ];
    if !note.trim().is_empty() {
        args.push("-m".to_string());
        args.push(note);
    }
    run_args_at_root_then(state, "git", args, move |code| {
        if code == Some(0) {
            state.git.stash_note.set(String::new());
        }
        after_git(state);
    });
}

pub fn stash_apply(state: AppState, index: u32) {
    stash_command(state, "apply", index);
}

pub fn stash_pop(state: AppState, index: u32) {
    stash_command(state, "pop", index);
}

pub fn stash_drop(state: AppState, index: u32) {
    stash_command(state, "drop", index);
}

fn stash_command(state: AppState, verb: &'static str, index: u32) {
    run_args_at_root_then(
        state,
        "git",
        vec![
            "stash".to_string(),
            verb.to_string(),
            format!("stash@{{{index}}}"),
        ],
        move |_| {
            // A popped or dropped stash may be the one opened below the
            // list, and `stash@{0}` now names a different one — or nothing.
            state.git.selected.set(None);
            state.git.detail.set(None);
            after_git(state);
        },
    );
}

/// Check a branch out. Through the command runner at the project root, so
/// the command and everything git says land in the dock where every other
/// command's do — a checkout that fails on a dirty tree has to be readable.
pub fn checkout(state: AppState, branch: String) {
    run_args_at_root_then(
        state,
        "git",
        vec!["checkout".to_string(), branch],
        move |_| after_git(state),
    );
}

/// Create a branch and switch to it, from `from` when given — the branch
/// selected in the strip — and from HEAD otherwise.
pub fn branch_create(state: AppState, name: String, from: Option<String>) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let mut args = vec!["checkout".to_string(), "-b".to_string(), name];
    if let Some(from) = from {
        args.push(from);
    }
    state.git.new_branch.set(None);
    state.git.branch_from.set(None);
    run_args_at_root_then(state, "git", args, move |_| {
        state.git.rev.set(None);
        after_git(state);
    });
}

/// Delete a local branch — the safe way. `-d` refuses a branch whose work
/// is not merged anywhere, and that refusal in the dock is the right answer;
/// `-D` is a decision to make with a terminal, not a button.
pub fn branch_delete(state: AppState, name: String) {
    run_args_at_root_then(
        state,
        "git",
        vec!["branch".to_string(), "-d".to_string(), name],
        move |_| {
            state.git.rev.set(None);
            after_git(state);
        },
    );
}

pub fn fetch(state: AppState) {
    run_args_at_root_then(
        state,
        "git",
        vec![
            "fetch".to_string(),
            "--all".to_string(),
            "--prune".to_string(),
        ],
        move |_| after_git(state),
    );
}

pub fn pull(state: AppState) {
    run_args_at_root_then(state, "git", vec!["pull".to_string()], move |_| {
        after_git(state)
    });
}

/// Push the current branch. With no upstream yet, set one on `origin` —
/// what the first push of a new branch wants, and what a bare `git push`
/// refuses with a hint nobody reads.
pub fn push(state: AppState) {
    let (head, upstream) = state.git.status.with_untracked(|s| {
        s.as_ref()
            .map(|s| (s.head.clone(), s.upstream.clone()))
            .unwrap_or((None, None))
    });
    let mut args = vec!["push".to_string()];
    if upstream.is_none()
        && let Some(head) = head
    {
        args.push("-u".to_string());
        args.push("origin".to_string());
        args.push(head);
    }
    run_args_at_root_then(state, "git", args, move |_| after_git(state));
}
