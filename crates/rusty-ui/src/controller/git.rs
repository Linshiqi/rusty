//! The repository: its history, a commit opened, and moving between branches.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_git::{Branch, CommitDetail, History};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
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
    }
    let args = Args {
        rev: state.git.rev.get_untracked(),
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

/// Both halves again. What the watcher calls after every batch, and the
/// refresh button calls on demand — a no-op until the panel has asked once,
/// so a project nobody has looked at the history of costs no `git log` per
/// save.
pub fn refresh_git(state: AppState) {
    if !state.git.loaded.get_untracked() {
        return;
    }
    load_history(state);
    load_branches(state);
}

/// Show one branch's history, or every branch's when `rev` is `None`.
pub fn show_rev(state: AppState, rev: Option<String>) {
    state.git.rev.set(rev);
    load_history(state);
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

/// Check a branch out. Through the command runner at the project root, so
/// the command and everything git says land in the dock where every other
/// command's do — a checkout that fails on a dirty tree has to be readable.
/// The tree and the open files follow the disk through the watcher; the log
/// and the branch strip are refreshed here once the command has finished.
pub fn checkout(state: AppState, branch: String) {
    run_command_at_root_then(state, format!("git checkout {branch}"), move |_| {
        refresh_git(state);
        refresh_tree(state);
    });
}
