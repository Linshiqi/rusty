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
    ChangeKind, CommitDetail, GitIdentity, GitOperation, GitStamp, History, Refs, Remote, Stash,
    Status,
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

mod changes;
mod clone;
mod history;
mod refs;

pub use changes::*;
pub use clone::*;
pub use history::*;
pub use refs::*;

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

/// Every remote the config names. Quiet on failure, like the refs; asked
/// when the panel opens, after every write, and when the stamp says the
/// config moved — a remote added in a terminal.
pub fn load_remotes(state: AppState) {
    if !state.has_project_now() || !begin(state, GitRead::Remotes) {
        return;
    }
    spawn_local(async move {
        if let Ok(remotes) = ipc::get::<Vec<Remote>>(cmd::git::REMOTES).await {
            set_if_changed(state.git.remotes, remotes);
        }
        if finish(state, GitRead::Remotes) {
            load_remotes(state);
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
    load_remotes(state);
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
        if stale.remotes {
            load_remotes(state);
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
    state.git.remotes.set(Vec::new());
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
    load_remotes(state);
    refresh_tree(state);
}

/// A `git` line in the dock, then `after_git`.
fn git(state: AppState, args: Vec<String>) {
    run_args_at_root_then(state, "git", args, move |_| after_git(state));
}

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

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

#[cfg(test)]
mod tests {
    use super::{checkout_args, is_hash, pick_remote};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_string()).collect()
    }

    /// Where a push goes: the upstream's remote, then `origin`, then any —
    /// and nowhere at all when there is none, which is when the panel asks
    /// for one instead of running a push git can only refuse.
    #[test]
    fn a_push_goes_to_the_remote_there_is_evidence_for() {
        assert_eq!(
            pick_remote(Some("fork/main"), &names(&["origin", "fork"])).as_deref(),
            Some("fork"),
            "the branch's own upstream decides first",
        );
        assert_eq!(
            pick_remote(Some("team/fw/main"), &names(&["team", "team/fw"])).as_deref(),
            Some("team/fw"),
            "the longest remote name that begins the upstream owns it",
        );
        assert_eq!(
            pick_remote(None, &names(&["upstream", "origin"])).as_deref(),
            Some("origin")
        );
        assert_eq!(
            pick_remote(None, &names(&["github"])).as_deref(),
            Some("github")
        );
        assert_eq!(pick_remote(None, &[]), None);
        assert_eq!(
            pick_remote(Some("origin/main"), &[]).as_deref(),
            Some("origin"),
            "an upstream is evidence of its remote before the list has arrived",
        );
    }

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
