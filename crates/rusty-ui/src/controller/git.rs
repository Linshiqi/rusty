//! The repository: its history, its working tree, its branches, tags and
//! stashes, and the commands that move any of them.
//!
//! Two kinds of call, on purpose. **Reads** — the log, the refs, a commit,
//! the status, the stash list, one path's diff — are IPC calls that answer
//! with model types and never touch the dock. **Writes** run as visible dock
//! commands through the same runner every `cargo` and `espflash` uses, so
//! the exact `git` line and everything git says back are readable, and a
//! failure on a dirty tree or a rejected push is a paragraph in the dock
//! rather than a banner nobody can act on. The one exception is staging:
//! `git add` on a file is instant and reversible, and a dock line per click
//! would bury the commands that matter under the ones that do not.
//!
//! **Read only what moved, and draw only what changed.** The panel used to
//! re-read everything — nine `git` processes — after every save anywhere in
//! the project, and set every signal whether or not the answer differed, so
//! the whole history was rebuilt each time. Now a save re-reads the status
//! alone; anything else is decided by the repository's stamp
//! (`rusty_git::GitStamp`), which costs no `git` at all and is also asked
//! every few seconds while the panel is showing — how a commit made in a
//! terminal, which the file watcher cannot see, reaches the panel. Every
//! answer is compared with what is on screen before it is set, and one of
//! each read is in flight at a time (`state::ReadGate`).

use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_git::{
    ChangeKind, CommitDetail, GitIdentity, GitOperation, GitStamp, History, Refs, Stash, Status,
};
use rusty_i18n::t;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{
        AppState, CloneDraft, GitMode, GitRead, ImagePair, ImageSide, ImageSource, PromptKind,
        RefPrompt, remember_split,
    },
};

/// How many opened commits are kept for a second look.
const CACHED_COMMITS: usize = 32;

fn begin(state: AppState, read: GitRead) -> bool {
    let mut go = false;
    state.git.gate.update_value(|gate| go = gate.begin(read));
    go
}

fn finish(state: AppState, read: GitRead) -> bool {
    let mut again = false;
    state
        .git
        .gate
        .update_value(|gate| again = gate.finish(read));
    again
}

// ─── reads ───────────────────────────────────────────────────────────────────

/// The log for the chosen branch, or for every branch.
///
/// Not through `track`: a project that is not a repository is an ordinary
/// thing to open, and "not inside a git repository" belongs in the panel
/// that asked, not on the banner over the whole window. An answer for a
/// filter or a length no longer asked for is dropped and asked again.
pub fn load_history(state: AppState) {
    if !state.has_project_now() || !begin(state, GitRead::History) {
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
    let asked = (args.rev.clone(), args.limit);
    spawn_local(async move {
        let answer = ipc::call::<_, History>(cmd::git::HISTORY, &args).await;
        let again = finish(state, GitRead::History);
        let current = (
            state.git.rev.get_untracked(),
            state.git.limit.get_untracked(),
        );
        if current == asked {
            match answer {
                Ok(history) => {
                    set_if_changed(state.git.history, Some(history));
                    set_if_changed(state.git.unavailable, None);
                    set_if_changed(state.git.not_a_repo, false);
                }
                Err(error) => {
                    state.git.history.set(None);
                    state
                        .git
                        .not_a_repo
                        .set(error.kind.as_deref() == Some("not-a-repository"));
                    state.git.unavailable.set(Some(error.message));
                }
            }
            set_if_changed(state.git.loaded, true);
        }
        if again || current != asked {
            load_history(state);
        }
    });
}

/// Every branch and tag. Quiet on failure: the history has already said
/// why, once.
pub fn load_refs(state: AppState) {
    if !state.has_project_now() || !begin(state, GitRead::Refs) {
        return;
    }
    spawn_local(async move {
        if let Ok(Refs { branches, tags }) = ipc::get::<Refs>(cmd::git::REFS).await {
            set_if_changed(state.git.branches, branches);
            set_if_changed(state.git.tags, tags);
        }
        if finish(state, GitRead::Refs) {
            load_refs(state);
        }
    });
}

/// Where the working tree stands. Quiet on failure, like the refs.
pub fn load_status(state: AppState) {
    if !state.has_project_now() || !begin(state, GitRead::Status) {
        return;
    }
    spawn_local(async move {
        if let Ok(status) = ipc::get::<Status>(cmd::git::STATUS).await {
            set_if_changed(state.git.status, Some(status));
        }
        if finish(state, GitRead::Status) {
            load_status(state);
        }
    });
}

pub fn load_stashes(state: AppState) {
    if !state.has_project_now() || !begin(state, GitRead::Stashes) {
        return;
    }
    spawn_local(async move {
        if let Ok(stashes) = ipc::get::<Vec<Stash>>(cmd::git::STASHES).await {
            set_if_changed(state.git.stashes, stashes);
        }
        if finish(state, GitRead::Stashes) {
            load_stashes(state);
        }
    });
}

/// Every read at once — the panel opening on a project, or the refresh
/// button. The stamp is taken beside them as the baseline later probes are
/// compared with; taken before the answers arrive, a change racing them is
/// seen again rather than missed.
pub fn load_git(state: AppState) {
    take_stamp(state);
    load_history(state);
    load_refs(state);
    load_status(state);
    load_stashes(state);
    load_identity(state);
}

fn take_stamp(state: AppState) {
    spawn_local(async move {
        if let Ok(stamp) = ipc::get::<GitStamp>(cmd::git::STAMP).await {
            state.git.stamp.set_value(Some(stamp));
        }
    });
}

/// Ask whether the repository moved since it was last read, and read again
/// only what did. No `git` runs for the question — see `GitStamp` — so this
/// is what the panel asks every few seconds while it is showing.
pub fn probe_git(state: AppState) {
    if !state.git.loaded.get_untracked() || !state.has_project_now() {
        return;
    }
    spawn_local(async move {
        let Ok(stamp) = ipc::get::<GitStamp>(cmd::git::STAMP).await else {
            return;
        };
        let before = state.git.stamp.get_value();
        state.git.stamp.set_value(Some(stamp));
        let Some(before) = before else {
            return;
        };
        let stale = stamp.stale_since(&before);
        if stale.history {
            load_history(state);
        }
        if stale.refs {
            load_refs(state);
        }
        if stale.status {
            load_status(state);
        }
        if stale.stashes {
            load_stashes(state);
        }
    });
}

/// What the file watcher calls after every batch — a no-op until the panel
/// has been opened once, so a project nobody has looked at the history of
/// costs nothing per save. A saved file changes the working tree and
/// nothing git keeps, so the status is re-read and the rest is left to the
/// stamp: a checkout or a pull in a terminal moves HEAD or the refs too.
pub fn refresh_git(state: AppState) {
    if !state.git.loaded.get_untracked() {
        return;
    }
    load_status(state);
    probe_git(state);
}

/// The panel was mounted. A project it has shown before is only probed:
/// the panel is rebuilt every time it is switched to, and it read
/// everything again and dropped the opened commit each time. A project it
/// has not shown starts clean.
pub fn open_git_panel(state: AppState, root: String) {
    let known = state
        .git
        .root
        .with_value(|shown| shown.as_deref() == Some(root.as_str()));
    if known && state.git.loaded.get_untracked() {
        probe_git(state);
        return;
    }
    state.git.root.set_value(Some(root));
    state.git.stamp.set_value(None);
    state.git.cache.set_value(Vec::new());
    state.git.history.set(None);
    state.git.branches.set(Vec::new());
    state.git.tags.set(Vec::new());
    state.git.status.set(None);
    state.git.stashes.set(Vec::new());
    state.git.unavailable.set(None);
    state.git.not_a_repo.set(false);
    state.git.loaded.set(false);
    state.git.selected.set(None);
    state.git.detail.set(None);
    state.git.file.set(None);
    state.git.rev.set(None);
    state.git.limit.set(rusty_git::LIMIT);
    state.git.diff.set(None);
    state.git.diff_for.set(None);
    state.git.prompt.set(None);
    state.git.query.set(String::new());
    state.git.reveal.set(None);
    state.git.amend.set(false);
    state.git.menu.set(None);
    load_git(state);
}

/// Who a commit would be signed as. Read with the rest of the panel and
/// again after the identity form, which is one of the writes.
pub fn load_identity(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    spawn_local(async move {
        if let Ok(identity) = ipc::call::<_, GitIdentity>(cmd::git::IDENTITY, &()).await {
            state.git.identity.set(Some(identity));
        }
    });
}

/// After any command that changed the repository: read everything back —
/// compared before it is drawn, so what did not move is not redrawn — take
/// a new baseline, and let the tree and the open files follow the disk.
fn after_git(state: AppState) {
    take_stamp(state);
    load_history(state);
    load_refs(state);
    load_status(state);
    load_stashes(state);
    refresh_tree(state);
}

/// A `git` line in the dock, then `after_git`.
fn git(state: AppState, args: Vec<String>) {
    run_args_at_root_then(state, "git", args, move |_| after_git(state));
}

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

// ─── identity and init ───────────────────────────────────────────────────────

/// `git config user.name` and `user.email` — for every repository unless
/// `local`, the two scopes git's own hint offers. Two dock commands, the
/// second after the first succeeds, then the identity is read back so the
/// form goes away on git's word rather than on ours.
pub fn set_identity(state: AppState, name: String, email: String, local: bool) {
    let scope = move || {
        if local {
            Vec::new()
        } else {
            vec!["--global".to_string()]
        }
    };
    let mut for_name = vec!["config".to_string()];
    for_name.extend(scope());
    for_name.extend(["user.name".to_string(), name]);
    let mut for_email = vec!["config".to_string()];
    for_email.extend(scope());
    for_email.extend(["user.email".to_string(), email]);
    run_args_at_root_then(state, "git", for_name, move |code| {
        if code != Some(0) {
            load_identity(state);
            return;
        }
        run_args_at_root_then(state, "git", for_email, move |_| load_identity(state));
    });
}

/// `git init` in the project, as a dock command, then read everything back:
/// an empty repository logs nothing and refuses nothing, so the panel comes
/// up with the working tree as untracked changes and no history — the
/// ordinary state of a project that has just started keeping one.
pub fn git_init(state: AppState) {
    run_args_at_root_then(state, "git", words(&["init"]), move |code| {
        if code == Some(0) {
            after_git(state);
        }
    });
}

// ─── the log ─────────────────────────────────────────────────────────────────

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

/// Whether a name is a full hash, which names one content for ever — the
/// only kind of name an opened commit is kept under. `stash@{0}` names a
/// different stash after every push or pop.
fn is_hash(id: &str) -> bool {
    matches!(id.len(), 40 | 64) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn cached(state: AppState, id: &str) -> Option<CommitDetail> {
    state
        .git
        .cache
        .with_value(|cache| cache.iter().find(|detail| detail.commit.id == id).cloned())
}

fn remember(state: AppState, detail: CommitDetail) {
    state.git.cache.update_value(|cache| {
        cache.retain(|kept| kept.commit.id != detail.commit.id);
        cache.insert(0, detail);
        cache.truncate(CACHED_COMMITS);
    });
}

/// Open a commit: its message, its files, their patches.
///
/// The commit on screen stays there, dimmed, until the new one arrives: it
/// used to be cleared first, and the pane collapsed to a strip and grew
/// back on every click. One seen before is shown at once, from the cache.
pub fn select_commit(state: AppState, id: String) {
    #[derive(serde::Serialize)]
    struct Args {
        id: String,
    }
    state.git.selected.set(Some(id.clone()));
    if let Some(detail) = cached(state, &id) {
        state.git.detail_loading.set(false);
        show_detail(state, detail);
        return;
    }
    state.git.detail_loading.set(true);
    let args = Args { id: id.clone() };
    track(
        state,
        async move {
            let answer = ipc::call::<_, CommitDetail>(cmd::git::COMMIT, &args).await;
            if answer.is_err() {
                state.git.detail_loading.set(false);
            }
            answer
        },
        move |detail| {
            // A later click wins: the answer to an earlier one arriving after
            // it must not replace what the user is looking at now.
            if state.git.selected.get_untracked().as_deref() != Some(id.as_str()) {
                return;
            }
            state.git.detail_loading.set(false);
            if is_hash(&id) {
                remember(state, detail.clone());
            }
            show_detail(state, detail);
        },
    );
}

/// Put an opened commit on screen, keeping the file that was showing when
/// the new commit touched it too — walking down the log reading one file's
/// history is what that is for — and the first file otherwise.
fn show_detail(state: AppState, detail: CommitDetail) {
    let kept = state
        .git
        .file
        .get_untracked()
        .filter(|path| detail.files.iter().any(|file| &file.path == path));
    let path = kept.or_else(|| detail.files.first().map(|file| file.path.clone()));
    state.git.detail.set(Some(detail));
    match path {
        Some(path) => show_commit_file(state, path),
        None => state.git.file.set(None),
    }
}

/// Move the selection `step` rows through the log — the arrow keys. The
/// row is selected and scrolled to at once; it is *opened* only once the
/// keys stop, since holding an arrow down passes rows faster than a commit
/// can be read.
pub fn select_step(state: AppState, step: i64) {
    let next = state.git.history.with_untracked(|history| {
        let rows = &history.as_ref()?.rows;
        if rows.is_empty() {
            return None;
        }
        let at = state.git.selected.with_untracked(|selected| {
            selected
                .as_ref()
                .and_then(|id| rows.iter().position(|row| &row.commit.id == id))
        });
        let target = match at {
            Some(at) => (at as i64 + step).clamp(0, rows.len() as i64 - 1) as usize,
            None => 0,
        };
        Some(rows[target].commit.id.clone())
    });
    let Some(id) = next else {
        return;
    };
    if state.git.selected.get_untracked().as_deref() == Some(id.as_str()) {
        return;
    }
    state.git.selected.set(Some(id.clone()));
    state.git.reveal.set(Some(id.clone()));
    set_timeout(
        move || {
            if state.git.selected.get_untracked().as_deref() == Some(id.as_str()) {
                select_commit(state, id);
            }
        },
        Duration::from_millis(140),
    );
}

/// Go to the commit a branch or a tag names: select it and scroll the log
/// to it. A tip older than the rows loaded, or on a branch the filter
/// hides, is shown by filtering the log to `rev` and selecting it there.
pub fn reveal_ref(state: AppState, id: String, rev: Option<String>) {
    if state.git.mode.get_untracked() != GitMode::History {
        state.git.mode.set(GitMode::History);
    }
    let loaded = state.git.history.with_untracked(|history| {
        history
            .as_ref()
            .is_some_and(|history| history.rows.iter().any(|row| row.commit.id == id))
    });
    if !loaded && let Some(rev) = rev {
        show_rev(state, Some(rev));
    }
    state.git.reveal.set(Some(id.clone()));
    select_commit(state, id);
}

/// The next row the search matches after the selected one, or the one
/// before it — Enter and Shift+Enter in the search box.
pub fn step_search(state: AppState, forward: bool) {
    let query = state.git.query.get_untracked();
    let target = state.git.history.with_untracked(|history| {
        let rows = &history.as_ref()?.rows;
        let hits = crate::gitlog::hits(rows, &query);
        let at = state.git.selected.with_untracked(|selected| {
            selected
                .as_ref()
                .and_then(|id| rows.iter().position(|row| &row.commit.id == id))
        });
        let hit = crate::gitlog::step_hit(&hits, at, forward)?;
        Some(rows[hit].commit.id.clone())
    });
    if let Some(id) = target {
        state.git.reveal.set(Some(id.clone()));
        select_commit(state, id);
    }
}

/// Show one of the opened commit's files — its patch, or, for an image, the
/// picture before and after. The two sides are the commit's first parent
/// and the commit itself, less whichever side an added or deleted file does
/// not have.
pub fn show_commit_file(state: AppState, path: String) {
    state.git.diff_whole.set(false);
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

// ─── clone ───────────────────────────────────────────────────────────────────

/// Open the clone dialog, empty.
pub fn open_clone_dialog(state: AppState) {
    state.git.clone.set(Some(CloneDraft::default()));
}

/// Ask the OS for the folder the clone lands in.
pub fn choose_clone_folder(state: AppState) {
    spawn_local(async move {
        match ipc::pick_folder(&t!("git.clone-into")).await {
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

// ─── the working tree ────────────────────────────────────────────────────────

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

// ─── commits ─────────────────────────────────────────────────────────────────

/// Check a commit or a tag out — a detached HEAD, said so in the dock.
pub fn checkout_commit(state: AppState, id: String) {
    git(state, words(&["checkout", "--detach", &id]));
}

/// Apply one commit's change on top of the current branch.
pub fn cherry_pick(state: AppState, id: String) {
    git(state, words(&["cherry-pick", &id]));
}

/// A new commit undoing an old one. `--no-edit` takes git's own message —
/// an editor would open on a terminal nobody is watching.
pub fn revert_commit(state: AppState, id: String) {
    git(state, words(&["revert", "--no-edit", &id]));
}

// ─── stashes ─────────────────────────────────────────────────────────────────

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

// ─── branches ────────────────────────────────────────────────────────────────

/// The remote a push or a new upstream goes to: the current branch's own
/// upstream's, else `origin` when there is one, else the first remote any
/// branch is on — and `origin` when nothing says, which is git's own
/// default name for the one it cloned from.
fn default_remote(state: AppState) -> String {
    let upstream_remote = state.git.status.with_untracked(|status| {
        status
            .as_ref()
            .and_then(|s| s.upstream.as_deref())
            .and_then(|upstream| upstream.split('/').next())
            .map(str::to_string)
    });
    if let Some(remote) = upstream_remote {
        return remote;
    }
    state.git.branches.with_untracked(|branches| {
        let remotes: Vec<&str> = branches
            .iter()
            .filter_map(|b| b.remote_name.as_deref())
            .collect();
        if remotes.contains(&"origin") {
            "origin".to_string()
        } else {
            remotes
                .first()
                .map(|remote| (*remote).to_string())
                .unwrap_or_else(|| "origin".to_string())
        }
    })
}

/// The arguments that check out `name`. A local branch by name; a remote
/// one through the local branch of the same name when there is one, and a
/// new local branch tracking it otherwise — checking a remote branch out by
/// its own name would detach HEAD, which is never what a double-click on
/// `origin/feature` means.
pub(crate) fn checkout_args(
    name: &str,
    remote: bool,
    local: &str,
    local_exists: bool,
) -> Vec<String> {
    if !remote {
        words(&["checkout", name])
    } else if local_exists {
        words(&["checkout", local])
    } else {
        words(&["checkout", "--track", name])
    }
}

/// Check a branch out. Through the dock, so a checkout refused on a dirty
/// tree is readable there.
pub fn checkout_branch(state: AppState, name: String, remote: bool) {
    let (local, exists) = state.git.branches.with_untracked(|branches| {
        let local = branches
            .iter()
            .find(|b| b.name == name)
            .map(|b| b.local_name().to_string())
            .unwrap_or_else(|| name.clone());
        let exists = branches.iter().any(|b| !b.remote && b.name == local);
        (local, exists)
    });
    let args = checkout_args(&name, remote, &local, exists);
    run_args_at_root_then(state, "git", args, move |_| {
        // The filter was the branch being left, or the one arrived at; either
        // way the whole graph is the useful view after a switch.
        state.git.rev.set(None);
        after_git(state);
    });
}

/// Open the name field.
pub fn open_prompt(state: AppState, kind: PromptKind) {
    let value = match &kind {
        PromptKind::Rename { from } => from.clone(),
        _ => String::new(),
    };
    state.git.prompt.set(Some(RefPrompt { kind, value }));
}

/// Carry out the name field: make the branch, rename one, or make the tag.
/// A name git would refuse is not sent — the field says why instead.
pub fn submit_prompt(state: AppState) {
    let Some(prompt) = state.git.prompt.get_untracked() else {
        return;
    };
    let name = prompt.value.trim().to_string();
    if rusty_git::ref_name_problem(&name).is_some() {
        return;
    }
    state.git.prompt.set(None);
    match prompt.kind {
        PromptKind::Branch { from } => {
            let mut args = words(&["checkout", "-b", &name]);
            args.extend(from);
            run_args_at_root_then(state, "git", args, move |_| {
                state.git.rev.set(None);
                after_git(state);
            });
        }
        PromptKind::Rename { from } => {
            if from != name {
                git(state, words(&["branch", "-m", &from, &name]));
            }
        }
        PromptKind::Tag { at } => git(state, words(&["tag", &name, &at])),
    }
}

/// Delete a local branch — the safe way. `-d` refuses a branch whose work
/// is not merged anywhere, and that refusal in the dock is the right answer;
/// `-D` is a decision to make with a terminal, not a button.
pub fn delete_branch(state: AppState, name: String) {
    run_args_at_root_then(state, "git", words(&["branch", "-d", &name]), move |_| {
        if state.git.rev.get_untracked().as_deref() == Some(name.as_str()) {
            state.git.rev.set(None);
        }
        after_git(state);
    });
}

/// Delete a branch on its remote — everybody's copy, so it asks first.
pub fn delete_remote_branch(state: AppState, name: String) {
    let (remote, branch) = state.git.branches.with_untracked(|branches| {
        branches
            .iter()
            .find(|b| b.name == name)
            .map(|b| {
                (
                    b.remote_name
                        .clone()
                        .unwrap_or_else(|| "origin".to_string()),
                    b.local_name().to_string(),
                )
            })
            .unwrap_or_else(|| ("origin".to_string(), name.clone()))
    });
    let question = t!("git.delete-remote-confirm", name = name.clone());
    spawn_local(async move {
        if ipc::confirm(&question).await {
            git(state, words(&["push", &remote, "--delete", &branch]));
        }
    });
}

/// Merge a branch into the one checked out. `--no-edit` takes git's own
/// message; a conflict stops it, and the panel's banner then offers the way
/// on and the way back.
pub fn merge_into_current(state: AppState, name: String) {
    git(state, words(&["merge", "--no-edit", &name]));
}

/// Rebase the branch checked out onto another — it rewrites the current
/// branch's commits, so it asks first.
pub fn rebase_onto(state: AppState, name: String) {
    let current = state.git.branches.with_untracked(|branches| {
        branches
            .iter()
            .find(|b| b.current)
            .map(|b| b.name.clone())
            .unwrap_or_default()
    });
    let question = t!("git.rebase-confirm", current = current, onto = name.clone());
    spawn_local(async move {
        if ipc::confirm(&question).await {
            git(state, words(&["rebase", &name]));
        }
    });
}

/// Push a local branch that is not the one checked out: to its upstream's
/// remote, or with `-u` to the default remote when it has none yet.
pub fn push_branch(state: AppState, name: String) {
    let upstream = state.git.branches.with_untracked(|branches| {
        branches
            .iter()
            .find(|b| !b.remote && b.name == name)
            .and_then(|b| b.upstream.clone())
    });
    let args = match upstream.as_deref().and_then(|u| u.split('/').next()) {
        Some(remote) => words(&["push", remote, &name]),
        None => words(&["push", "-u", &default_remote(state), &name]),
    };
    git(state, args);
}

/// Push the current branch. With no upstream yet, set one — what the first
/// push of a new branch wants, and what a bare `git push` refuses with a
/// hint nobody reads.
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
        args.extend(words(&["-u", &default_remote(state), &head]));
    }
    git(state, args);
}

pub fn fetch(state: AppState) {
    git(state, words(&["fetch", "--all", "--prune"]));
}

pub fn pull(state: AppState) {
    git(state, words(&["pull"]));
}

// ─── tags ────────────────────────────────────────────────────────────────────

/// Delete a tag here. A pushed tag stays on the remote, which the question
/// says.
pub fn delete_tag(state: AppState, name: String) {
    let question = t!("git.delete-tag-confirm", name = name.clone());
    spawn_local(async move {
        if ipc::confirm(&question).await {
            git(state, words(&["tag", "-d", &name]));
        }
    });
}

/// Push one tag. By its full name, since a branch may share it.
pub fn push_tag(state: AppState, name: String) {
    let remote = default_remote(state);
    git(
        state,
        words(&["push", &remote, &format!("refs/tags/{name}")]),
    );
}

// ─── an operation left half done ─────────────────────────────────────────────

fn operation(state: AppState) -> Option<GitOperation> {
    state
        .git
        .status
        .with_untracked(|status| status.as_ref().and_then(|s| s.operation))
}

/// Carry on with the merge, rebase, cherry-pick or revert that stopped:
/// once the conflicts are resolved and staged, this is the commit it was
/// waiting for. No editor opens — the dock's git runs with `GIT_EDITOR=true`
/// — so the message git prepared is the one used.
pub fn continue_operation(state: AppState) {
    let args = match operation(state) {
        Some(GitOperation::Merge) => words(&["commit", "--no-edit"]),
        Some(GitOperation::Rebase) => words(&["rebase", "--continue"]),
        Some(GitOperation::CherryPick) => words(&["cherry-pick", "--continue"]),
        Some(GitOperation::Revert) => words(&["revert", "--continue"]),
        None => return,
    };
    git(state, args);
}

/// Go back to before the operation started. Conflicts already resolved are
/// thrown away with it, so it asks first.
pub fn abort_operation(state: AppState) {
    let verb = match operation(state) {
        Some(GitOperation::Merge) => "merge",
        Some(GitOperation::Rebase) => "rebase",
        Some(GitOperation::CherryPick) => "cherry-pick",
        Some(GitOperation::Revert) => "revert",
        None => return,
    };
    let question = t!("git.abort-confirm");
    spawn_local(async move {
        if ipc::confirm(&question).await {
            git(state, words(&[verb, "--abort"]));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{checkout_args, is_hash};

    #[test]
    fn a_remote_branch_is_checked_out_through_a_local_one() {
        assert_eq!(
            checkout_args("main", false, "main", true),
            ["checkout", "main"]
        );
        assert_eq!(
            checkout_args("origin/feature/x", true, "feature/x", true),
            ["checkout", "feature/x"],
            "the local branch of that name, when there is one"
        );
        assert_eq!(
            checkout_args("origin/feature/x", true, "feature/x", false),
            ["checkout", "--track", "origin/feature/x"],
            "a new one tracking it, when there is not — never a detached HEAD"
        );
    }

    #[test]
    fn only_a_full_hash_is_kept_as_a_name_for_one_content() {
        assert!(is_hash("20d12f8de4db7a9000627bf3c1d8ca9ecc8500db"));
        assert!(
            !is_hash("20d12f8"),
            "a short hash can come to name two commits"
        );
        assert!(
            !is_hash("stash@{0}"),
            "a stash's name moves with every push"
        );
        assert!(!is_hash("HEAD"));
    }
}
