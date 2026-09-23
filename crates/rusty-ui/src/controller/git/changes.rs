//! The working tree: a file's diff, staging, discarding, the commit and
//! the stashes.

use super::*;

/// One working-tree path's diff, for the Changes view.
pub fn load_diff(state: AppState, path: String, staged: bool, untracked: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        staged: bool,
        untracked: bool,
    }
    let key = (path.clone(), staged);
    state.git.diff_whole.set(false);
    state.git.diff_for.set(Some(key.clone()));
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
                set_if_changed(state.git.diff, Some(text));
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
        async move {
            let answer = ipc::call::<_, ()>(cmd::git::STAGE, &args).await;
            // Read back whichever way it went: a stage that failed on one
            // path still staged the others (`--ignore-errors`), and a list
            // that did not move would say they were not.
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
            answer
        },
        move |()| {},
    );
}

/// Fold an empty repository inside the project into it — asked first, since
/// its `.git` is removed (to the recycle bin). The backend checks again that
/// it has no commits.
pub fn include_nested(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    let question = t!("git.include-nested-confirm", path = path.clone());
    spawn_local(async move {
        if !ipc::confirm(&question).await {
            return;
        }
        let args = Args { path };
        track(
            state,
            async move { ipc::call::<_, ()>(cmd::git::INCLUDE_NESTED, &args).await },
            move |()| {
                load_status(state);
                refresh_tree(state);
            },
        );
    });
}

/// Throw a file's changes away — the one write in this panel that cannot be
/// undone, so it asks first, in words that say what happens to *this* file:
/// from the unstaged list the tree goes back to the index; from the staged
/// list the file goes back to the last commit in both; an untracked file has
/// nothing to go back to and is deleted (`clean -f` on that one path, never
/// `-d` or the tree). Fork's "Discard changes…".
pub fn discard(state: AppState, path: String, staged: bool, untracked: bool) {
    let question = if untracked {
        t!("git.discard-untracked-confirm", path = path.clone())
    } else if staged {
        t!("git.discard-staged-confirm", path = path.clone())
    } else {
        t!("git.discard-unstaged-confirm", path = path.clone())
    };
    let args = if untracked {
        words(&["clean", "-f", "--", &path])
    } else if staged {
        words(&[
            "restore",
            "--source=HEAD",
            "--staged",
            "--worktree",
            "--",
            &path,
        ])
    } else {
        words(&["restore", "--", &path])
    };
    // The answer arrives asynchronously in the app (a native dialog through
    // the dialog plugin) and synchronously in a browser; `ipc::confirm`
    // hides the difference. The first version read `window.confirm` as a
    // boolean, which in the app is a Promise and therefore false: the menu
    // item did nothing, with no error anywhere.
    spawn_local(async move {
        if !ipc::confirm(&question).await {
            return;
        }
        forget_diff_of(state, &path);
        git(state, args);
    });
}

/// Open a file named by the Git panel in the editor — and show the editor.
/// The panel fills the working area, so a file opened behind it is a click
/// that appears to do nothing; VS Code's SCM view brings the editor forward
/// for the same reason.
pub fn open_from_git(state: AppState, path: String) {
    open_file(state.focused(), path);
    state.layout.panel.set("files".to_string());
}

/// One file into a stash, index and tree both, untracked included — Fork's
/// "Stash 1 File".
pub fn stash_file(state: AppState, path: String) {
    forget_diff_of(state, &path);
    git(
        state,
        words(&["stash", "push", "--include-untracked", "--", &path]),
    );
}

/// The diff pane shows one path; if that path is about to stop being a
/// change, the pane must not keep describing it.
fn forget_diff_of(state: AppState, path: &str) {
    if state
        .git
        .diff_for
        .with_untracked(|d| d.as_ref().is_some_and(|(p, _)| p == path))
    {
        state.git.diff_for.set(None);
        state.git.diff.set(None);
        state.git.images.set(None);
    }
}

/// Commit what is staged, with the message being written — or amend the
/// last commit with it. The message is one argument however many lines it
/// has: through the argument-vector runner, never the line splitter.
///
/// Only an amend may go without a message; it then keeps the one it has
/// (`--no-edit`) rather than opening an editor nobody can see.
pub fn commit(state: AppState) {
    // No author, no commit: git would refuse with "Author identity unknown",
    // and the form the box shows in that state is the answer to it.
    if state
        .git
        .identity
        .with_untracked(|id| id.as_ref().is_some_and(|id| !id.complete()))
    {
        return;
    }
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

/// Stash the working tree, untracked files included — "everything I have"
/// is what the button says — with the note if one was written.
pub fn stash_save(state: AppState) {
    let note = state.git.stash_note.get_untracked();
    let mut args = words(&["stash", "push", "--include-untracked"]);
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
    let args = vec![
        "stash".to_string(),
        verb.to_string(),
        format!("stash@{{{index}}}"),
    ];
    run_args_at_root_then(state, "git", args, move |_| {
        // A popped or dropped stash may be the one opened below the
        // list, and `stash@{0}` now names a different one — or nothing.
        state.git.selected.set(None);
        state.git.detail.set(None);
        after_git(state);
    });
}
