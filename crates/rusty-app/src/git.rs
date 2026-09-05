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

use base64::Engine;
use rusty_embed::{CommandPlan, LogLine, process};
use rusty_git::{Branch, CommitDetail, History, Stash, Status};
use tauri::{State, ipc::Channel};

use crate::{
    error::CommandError,
    state::{AppState, blocking},
    stream,
};

type Answer<T> = Result<T, CommandError>;

/// The log for `rev`, or every branch when `rev` is `None`, laid out. The
/// newest `limit` commits — the panel's default, doubled each time the user
/// asks for older ones, and capped here so a runaway caller cannot ask for a
/// hundred thousand rows of SVG.
#[tauri::command]
pub async fn git_history(
    rev: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Answer<History> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let limit = limit.unwrap_or(rusty_git::repo::LIMIT).clamp(1, 10_000);
    Ok(blocking("git log", move || {
        rusty_git::repo::history(&root, rev.as_deref(), limit)
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

/// One file's bytes at `spec` (a hash, `HEAD`, `:0` for the index) or in
/// the working tree when `spec` is absent, as base64 — the panel turns it
/// into a `data:` URL and shows the picture. Base64 rather than a byte array
/// because a JS array of a megabyte of numbers is the slow way across IPC.
#[tauri::command]
pub async fn git_blob(
    spec: Option<String>,
    path: String,
    state: State<'_, AppState>,
) -> Answer<String> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let bytes = blocking("git show", move || {
        rusty_git::repo::blob(&root, spec.as_deref(), &path)
    })
    .await??;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// `git clone <url> <into>`, streamed to the dock like an install step. The
/// one git command that runs with no project open, because it is how a
/// project arrives; the frontend opens `into` once the exit code is zero.
#[tauri::command]
pub async fn git_clone(
    url: String,
    into: String,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Answer<Option<i32>> {
    let step = CommandPlan {
        program: "git".to_string(),
        args: vec![
            "clone".to_string(),
            "--progress".to_string(),
            url.clone(),
            into.clone(),
        ],
        display: format!("git clone --progress {url} {into}"),
        rationale: "clones the repository into the chosen folder; the project opens when \
                    it finishes"
            .to_string(),
        warning: None,
    };
    crate::simulate::note(&on_line, format!("$ {}", step.display));
    let session = process::spawn(&step, None)?;
    let ours = state.start_session(session.stopper()).await;
    let feed = on_line.clone();
    let code = blocking("git clone", move || {
        stream::forward(|| session.recv(), &feed);
        session.wait()
    })
    .await?;
    state.release_session(&ours).await;
    Ok(code)
}

/// A commit in a window of its own — Fork's tear-off. The window boots the
/// same frontend with `?gitdiff=<target>` and shows only that commit's pane;
/// `target` is a hash or a `stash@{n}` name, whatever `git show` accepts.
#[tauri::command]
pub async fn open_git_window(target: String, app: tauri::AppHandle) -> Answer<()> {
    use tauri::Manager;

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in target.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let label = format!("git-{hash:x}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(());
    }
    let short: String = target.chars().take(7).collect();
    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(
            format!("index.html?gitdiff={}", crate::files::query_encode(&target)).into(),
        ),
    )
    .title(format!("{short} — rusty"))
    .inner_size(1100.0, 760.0)
    .build()
    .map_err(|error| CommandError::new(format!("could not open the commit window: {error}")))?;
    Ok(())
}
