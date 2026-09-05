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

use rusty_git::{Branch, ChangeKind, CommitDetail, History, Stash, Status};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, CloneDraft, ImagePair, ImageSide, ImageSource, remember_split},
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
            let first = detail.files.first().map(|f| f.path.clone());
            state.git.detail.set(Some(detail));
            match first {
                Some(path) => show_commit_file(state, path),
                None => state.git.file.set(None),
            }
        },
    );
}

/// Show one of the opened commit's files — its patch, or, for an image, the
/// picture before and after. The two sides are the commit's first parent
/// and the commit itself, less whichever side an added or deleted file does
/// not have.
pub fn show_commit_file(state: AppState, path: String) {
    state.git.file.set(Some(path.clone()));
    if !rusty_git::is_image_path(&path) {
        return;
    }
    let Some(detail) = state.git.detail.get_untracked() else {
        return;
    };
    let kind = detail.files.iter().find(|f| f.path == path).map(|f| f.kind);
    let old = match (kind, detail.commit.parents.first()) {
        (Some(ChangeKind::Added), _) | (_, None) => None,
        (_, Some(parent)) => Some(ImageSource::Rev(parent.clone())),
    };
    let new = match kind {
        Some(ChangeKind::Deleted) => None,
        _ => Some(ImageSource::Rev(detail.commit.id.clone())),
    };
    load_images(state, path, old, new);
}

/// Fetch an image's two sides as `data:` URLs. Each side answers on its own,
/// so a missing old side never delays the new one, and an answer for a
/// picture no longer showing is dropped.
pub fn load_images(
    state: AppState,
    path: String,
    old: Option<ImageSource>,
    new: Option<ImageSource>,
) {
    #[derive(serde::Serialize)]
    struct Args {
        spec: Option<String>,
        path: String,
    }
    let side = |source: &Option<ImageSource>| match source {
        None => ImageSide::Absent,
        Some(_) => ImageSide::Loading,
    };
    state.git.images.set(Some(ImagePair {
        path: path.clone(),
        old: side(&old),
        new: side(&new),
    }));
    let mime = rusty_git::image_mime(&path).unwrap_or("application/octet-stream");
    for (is_old, source) in [(true, old), (false, new)] {
        let Some(source) = source else {
            continue;
        };
        let args = Args {
            spec: match source {
                ImageSource::Worktree => None,
                ImageSource::Rev(rev) => Some(rev),
            },
            path: path.clone(),
        };
        let path = path.clone();
        spawn_local(async move {
            let side = match ipc::call::<_, String>(cmd::git::BLOB, &args).await {
                Ok(base64) => ImageSide::Ready {
                    // Base64 is four characters for three bytes.
                    bytes: base64.trim_end_matches('=').len() * 3 / 4,
                    url: format!("data:{mime};base64,{base64}"),
                },
                Err(error) => ImageSide::Failed(error.message),
            };
            state.git.images.update(|pair| {
                if let Some(pair) = pair
                    && pair.path == path
                {
                    if is_old {
                        pair.old = side;
                    } else {
                        pair.new = side;
                    }
                }
            });
        });
    }
}

/// Fold the opened commit's pane down to a strip, or bring it back.
pub fn toggle_detail(state: AppState) {
    state.git.detail_hidden.update(|hidden| *hidden = !*hidden);
}

/// Tear the opened commit off into a window of its own.
pub fn open_commit_window(state: AppState, target: String) {
    #[derive(serde::Serialize)]
    struct Args {
        target: String,
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::git::WINDOW, &Args { target }).await },
        |()| {},
    );
}

/// Open the clone dialog, empty.
pub fn open_clone_dialog(state: AppState) {
    state.git.clone.set(Some(CloneDraft::default()));
}

/// Ask the OS for the folder the clone lands in.
pub fn choose_clone_folder(state: AppState) {
    spawn_local(async move {
        match ipc::pick_folder(&rusty_i18n::t!("git.clone-into")).await {
            Ok(Some(folder)) => state.git.clone.update(|draft| {
                if let Some(draft) = draft {
                    draft.into = Some(folder);
                }
            }),
            Ok(None) => {}
            Err(error) => state.app.error.set(Some(error)),
        }
    });
}

/// Run the clone the dialog describes: into `<folder>/<name>`, where `name`
/// is what `git clone` itself would pick, streamed to the dock; the project
/// opens when git exits zero. The dialog stays up while it runs, so a second
/// click cannot start a second clone into the same directory.
pub fn clone_repository(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        url: String,
        into: String,
    }
    let Some(draft) = state.git.clone.get_untracked() else {
        return;
    };
    let url = draft.url.trim().to_string();
    let (Some(folder), Some(name)) = (draft.into.clone(), rusty_git::repo_name(&url)) else {
        return;
    };
    if draft.running {
        return;
    }
    let separator = if folder.contains('\\') && !folder.contains('/') {
        '\\'
    } else {
        '/'
    };
    let into = format!("{}{separator}{name}", folder.trim_end_matches(['/', '\\']));
    state.git.clone.update(|draft| {
        if let Some(draft) = draft {
            draft.running = true;
        }
    });
    state.dock.source.set("commands");
    let channel = stream_to_terminal(state);
    let args = Args {
        url,
        into: into.clone(),
    };
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, Option<i32>>(cmd::git::CLONE, &args, "onLine", &channel).await
        },
        move |code| {
            note_exit(state, code);
            if code == Some(0) {
                state.git.clone.set(None);
                open_project(state, into);
            } else {
                state.git.clone.update(|draft| {
                    if let Some(draft) = draft {
                        draft.running = false;
                    }
                });
            }
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
    // An image is compared as pictures: what was committed against the index
    // for a staged change, the index against the disk for an unstaged one —
    // and a file git has never seen has no old side at all.
    if rusty_git::is_image_path(&path) {
        let (old, new) = if staged {
            (
                Some(ImageSource::Rev("HEAD".to_string())),
                Some(ImageSource::Rev(":0".to_string())),
            )
        } else if untracked {
            (None, Some(ImageSource::Worktree))
        } else {
            (
                Some(ImageSource::Rev(":0".to_string())),
                Some(ImageSource::Worktree),
            )
        };
        load_images(state, path.clone(), old, new);
    }
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
