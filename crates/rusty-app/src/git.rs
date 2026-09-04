//! The repository's commands: history, one commit, the branches.
//!
//! Thin, like every other module here — `rusty_git::repo` runs git and reads
//! it; this hands it the opened project and crosses onto a blocking thread,
//! because `git log` on a large repository is a real fraction of a second and
//! an async worker must not sit through it.
//!
//! Always the opened project, never `firmware_root`: the repository is the
//! user's checkout, and the standard embedded layout puts the firmware crate
//! *inside* it, not the other way round.

use rusty_git::{Branch, CommitDetail, History, Stash, Status};
use tauri::State;

use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

type Answer<T> = Result<T, CommandError>;

/// The log for `rev`, or every branch when `rev` is `None`, laid out.
#[tauri::command]
pub async fn git_history(rev: Option<String>, state: State<'_, AppState>) -> Answer<History> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("git log", move || {
        rusty_git::repo::history(&root, rev.as_deref(), rusty_git::repo::LIMIT)
    })
    .await??)
}

/// One commit: message, files, each file's patch.
#[tauri::command]
pub async fn git_commit(id: String, state: State<'_, AppState>) -> Answer<CommitDetail> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("git show", move || rusty_git::repo::commit(&root, &id)).await??)
}

/// Local and remote-tracking branches, the current one marked.
#[tauri::command]
pub async fn git_branches(state: State<'_, AppState>) -> Answer<Vec<Branch>> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("git branch", move || rusty_git::repo::branches(&root)).await??)
}

/// Where the working tree stands.
#[tauri::command]
pub async fn git_status(state: State<'_, AppState>) -> Answer<Status> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("git status", move || rusty_git::repo::status(&root)).await??)
}

/// Every stash, newest first.
#[tauri::command]
pub async fn git_stashes(state: State<'_, AppState>) -> Answer<Vec<Stash>> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("git stash list", move || rusty_git::repo::stashes(&root)).await??)
}

/// One path's diff, for the Changes view.
#[tauri::command]
pub async fn git_diff(
    path: String,
    staged: bool,
    untracked: bool,
    state: State<'_, AppState>,
) -> Answer<String> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("git diff", move || {
        rusty_git::repo::diff_file(&root, &path, staged, untracked)
    })
    .await??)
}

/// Stage paths (`on`), or take them back out of the index.
#[tauri::command]
pub async fn git_stage(paths: Vec<String>, on: bool, state: State<'_, AppState>) -> Answer<()> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("git add", move || {
        if on {
            rusty_git::repo::stage(&root, &paths)
        } else {
            rusty_git::repo::unstage(&root, &paths)
        }
    })
    .await??)
}
