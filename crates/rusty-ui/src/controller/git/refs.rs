//! Branches, remotes and tags, the commands that move them, and a merge
//! or rebase that stopped halfway.

use super::*;

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

/// The remote a push or a new upstream goes to — `None` when the repository
/// has none, which is the case a push has to stop and ask about.
///
/// The current branch's upstream's remote first, then `origin`, then the
/// first there is. `known` is every remote name there is evidence of: the
/// config's, and the ones remote-tracking branches name, so a click in the
/// moment before the remotes have been read does not ask to add a remote
/// that plainly exists.
pub(crate) fn pick_remote(upstream: Option<&str>, known: &[String]) -> Option<String> {
    if let Some(upstream) = upstream {
        // The longest name that begins the upstream: a remote may be called
        // `team` and another `team/fw`, and `team/fw/main` is the second's.
        let owner = known
            .iter()
            .filter(|name| {
                upstream
                    .strip_prefix(name.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|name| name.len());
        return owner
            .cloned()
            .or_else(|| upstream.split('/').next().map(str::to_string));
    }
    known
        .iter()
        .find(|name| *name == "origin")
        .or_else(|| known.first())
        .cloned()
}

fn default_remote(state: AppState) -> Option<String> {
    let upstream = state
        .git
        .status
        .with_untracked(|status| status.as_ref().and_then(|s| s.upstream.clone()));
    let mut known: Vec<String> = state
        .git
        .remotes
        .with_untracked(|remotes| remotes.iter().map(|r| r.name.clone()).collect());
    state.git.branches.with_untracked(|branches| {
        for name in branches.iter().filter_map(|b| b.remote_name.clone()) {
            if !known.contains(&name) {
                known.push(name);
            }
        }
    });
    pick_remote(upstream.as_deref(), &known)
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

/// Open the name field — filled with what is being changed, or, for a new
/// remote, with `origin` while the repository has none by that name: it is
/// the name every guide, every host's instructions and git itself use for
/// the one a repository was cloned from or first pushed to.
pub fn open_prompt(state: AppState, kind: PromptKind) {
    let remotes = state.git.remotes.get_untracked();
    let value = match &kind {
        PromptKind::Rename { from } | PromptKind::RenameRemote { from } => from.clone(),
        PromptKind::Remote { .. } if !remotes.iter().any(|r| r.name == "origin") => {
            "origin".to_string()
        }
        PromptKind::RemoteUrl { name } => remotes
            .iter()
            .find(|r| &r.name == name)
            .map(|r| r.url.clone())
            .unwrap_or_default(),
        _ => String::new(),
    };
    state.git.prompt.set(Some(RefPrompt {
        kind,
        value,
        url: String::new(),
    }));
}

/// Carry out the field: make the branch, rename one, make the tag, or add,
/// rename or re-point a remote. Anything git would refuse is not sent — the
/// field says why instead.
///
/// The remote commands put `--` before what was typed. A name is already
/// held to a branch's rules, which refuse a leading `-`; the URL is refused
/// one too; and the `--` is what stays true if either rule is ever relaxed.
pub fn submit_prompt(state: AppState) {
    let Some(prompt) = state.git.prompt.get_untracked() else {
        return;
    };
    let remotes = state.git.remotes.get_untracked();
    if prompt.problem(&remotes).is_some() {
        return;
    }
    let name = prompt.value.trim().to_string();
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
        PromptKind::Remote { push } => {
            let url = prompt.url.trim().to_string();
            let args = words(&["remote", "add", "--", &name, &url]);
            run_args_at_root_then(state, "git", args, move |code| {
                after_git(state);
                // The remote list is being read again, and will not have
                // arrived by the time this line runs — so the push is told
                // where to go rather than left to look it up.
                if push && code == Some(0) {
                    push_current_to(state, name);
                }
            });
        }
        PromptKind::RenameRemote { from } => {
            if from != name {
                git(state, words(&["remote", "rename", "--", &from, &name]));
            }
        }
        PromptKind::RemoteUrl { name: remote } => {
            git(state, words(&["remote", "set-url", "--", &remote, &name]));
        }
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
        None => match default_remote(state) {
            Some(remote) => words(&["push", "-u", &remote, &name]),
            None => return open_prompt(state, PromptKind::Remote { push: false }),
        },
    };
    git(state, args);
}

/// Push the current branch. With no upstream yet, set one — what the first
/// push of a new branch wants, and what a bare `git push` refuses with a
/// hint nobody reads. With no remote at all there is nowhere to push to, so
/// the remote form opens instead and the push carries on once it is added:
/// the first push of a repository made with `git init` used to be `push -u
/// origin main` and git's "'origin' does not appear to be a git repository".
pub fn push(state: AppState) {
    let tracked = state
        .git
        .status
        .with_untracked(|s| s.as_ref().is_some_and(|s| s.upstream.is_some()));
    if tracked {
        return git(state, words(&["push"]));
    }
    match default_remote(state) {
        Some(remote) => push_current_to(state, remote),
        None => open_prompt(state, PromptKind::Remote { push: true }),
    }
}

/// Push the branch checked out to `remote` and make it the upstream — or a
/// plain push when an upstream already exists, which the remote form's
/// follow-up cannot know until the status is read.
fn push_current_to(state: AppState, remote: String) {
    let (head, upstream) = state.git.status.with_untracked(|s| {
        s.as_ref()
            .map(|s| (s.head.clone(), s.upstream.clone()))
            .unwrap_or((None, None))
    });
    let mut args = words(&["push"]);
    if upstream.is_none()
        && let Some(head) = head
    {
        args.extend(words(&["-u", &remote, &head]));
    }
    git(state, args);
}

/// Fetch one remote, dropping the branches it no longer has.
pub fn fetch_remote(state: AppState, name: String) {
    git(state, words(&["fetch", "--prune", "--", &name]));
}

/// Remove a remote. It asks first, and says what goes: this repository's
/// copies of the remote's branches and the upstreams that named them — and
/// nothing on the remote itself.
pub fn remove_remote(state: AppState, name: String) {
    let question = t!("git.remote-remove-confirm", name = name.clone());
    spawn_local(async move {
        if !ipc::confirm(&question).await {
            return;
        }
        let args = words(&["remote", "remove", "--", &name]);
        run_args_at_root_then(state, "git", args, move |_| {
            let prefix = format!("{name}/");
            if state
                .git
                .rev
                .get_untracked()
                .is_some_and(|rev| rev.starts_with(&prefix))
            {
                state.git.rev.set(None);
            }
            after_git(state);
        });
    });
}

pub fn fetch(state: AppState) {
    git(state, words(&["fetch", "--all", "--prune"]));
}

pub fn pull(state: AppState) {
    git(state, words(&["pull"]));
}

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
    let Some(remote) = default_remote(state) else {
        return open_prompt(state, PromptKind::Remote { push: false });
    };
    git(
        state,
        words(&["push", &remote, &format!("refs/tags/{name}")]),
    );
}

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
