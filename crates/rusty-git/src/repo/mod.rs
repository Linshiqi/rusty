//! Running `git` in the project and reading its answers.
//!
//! **One process per question.** Every call runs in the opened project, with
//! the user's own `git`: their config, their credentials, their hooks. And
//! every one costs about sixty milliseconds to *start* on Windows before it
//! reads a byte, so the count is the budget: the log is one process, the
//! refs one, the status one, the stash list one, a commit opened is one, and
//! the remotes are one — asked only when the config has moved, since a
//! remote changes far less often than anything else here.
//! There used to be a `rev-parse` in front of each of them to ask "is this a
//! repository?" — asked now only when a command has already failed, where
//! the answer decides which refusal to give.
//!
//! [`stamp`](fn@stamp) runs no `git` at all: it reads the sizes and times
//! of the files git keeps its state in, so the panel can ask "did anything
//! move?" every few seconds for nothing.

mod run;
mod stamp;
#[cfg(test)]
mod tests;

use std::path::Path;

use crate::graph;
use crate::model::{CommitDetail, GitOperation, History, RefKind, Refs, Remote, Stash, Status};
use crate::parse;
use run::{run, run_allowing, run_bytes};
use stamp::dirs;

pub use stamp::stamp;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not run git: {source}")]
    Spawn {
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not inside a git repository")]
    NotARepository { path: String },
    #[error("git {command} failed: {detail}")]
    Git { command: String, detail: String },
    #[error("no commit {id}")]
    NoSuchCommit { id: String },
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

pub use crate::model::LIMIT;

/// The log, laid out, for `rev` (`--all` when `None`), newest first.
pub fn history(root: &Path, rev: Option<&str>, limit: usize) -> Result<History> {
    let mut args = vec![
        "log".to_string(),
        "--topo-order".to_string(),
        // Full ref names in `%D`, so a local `feature/x` is not read as a
        // remote branch — see `parse::decorations`.
        "--decorate=full".to_string(),
        format!("--format={}", parse::LOG_FORMAT),
        // One more than asked, so "there are older commits" is a fact seen
        // rather than inferred from hitting the limit exactly.
        format!("-n{}", limit + 1),
    ];
    match rev {
        Some(rev) => args.push(rev.to_string()),
        // Every ref but the stash. `--all` reaches `refs/stash`, and a stash
        // is a merge commit whose parents — `index on main: …`, `untracked
        // files on main: …` — are commits too, so one `git stash` put three
        // rows and two extra lanes into the graph that no branch owns. The
        // Stashes view is where a stash is read; the history is for the
        // branches. `--exclude` applies to the `--all` that follows it.
        None => {
            args.push("--exclude=refs/stash".to_string());
            args.push("--all".to_string());
        }
    }
    let text = run(root, &args)?;
    let mut commits = parse::log(&text);
    let truncated = commits.len() > limit;
    commits.truncate(limit);
    let mut laid_out = graph::lay_out(commits);
    laid_out.truncated = truncated;
    laid_out.head = head_of(&laid_out);
    Ok(laid_out)
}

/// What HEAD names, off the log's own decorations: a branch, or the short
/// hash of the row HEAD sits on when it is detached. Two `git` calls asked
/// this before; the log already said it, whenever HEAD is among its rows.
fn head_of(history: &History) -> Option<String> {
    history.rows.iter().find_map(|row| {
        row.commit
            .refs
            .iter()
            .find(|label| label.kind == RefKind::Head)
            .map(|label| {
                if label.name.is_empty() {
                    row.commit.short.clone()
                } else {
                    label.name.clone()
                }
            })
    })
}

/// One commit opened: message, files, and each file's patch — one `git show`.
///
/// `-m --first-parent` so a merge shows what it brought in against its first
/// parent, the way Fork does, rather than the empty combined diff `git show`
/// prints for a clean merge.
pub fn commit(root: &Path, id: &str) -> Result<CommitDetail> {
    let text = run(
        root,
        &[
            "show",
            &format!("--format={}", parse::DETAIL_FORMAT),
            "--decorate=full",
            "-m",
            "--first-parent",
            "--raw",
            "--numstat",
            "-p",
            "--no-color",
            id,
        ],
    )
    .map_err(|error| match error {
        Error::NotARepository { .. } | Error::Spawn { .. } => error,
        _ => Error::NoSuchCommit { id: id.to_string() },
    })?;
    let mut detail =
        parse::detail(&text).ok_or_else(|| Error::NoSuchCommit { id: id.to_string() })?;
    // A stash saved with `--include-untracked` keeps those files in a third
    // parent, which the first-parent diff never reaches: a stash of one new
    // file opened as "no files changed" while `git stash list` plainly held
    // it. That parent is a root commit, so showing it lists every file it
    // holds as added — which is what they are, to the stash. Asked only of a
    // commit with a third parent, which an ordinary commit never has.
    if detail.commit.parents.len() >= 3
        && let Some(untracked) = untracked_parent(root, id)
    {
        let more = run(
            root,
            &[
                "show",
                "--format=",
                "--raw",
                "--numstat",
                "-p",
                "--no-color",
                &untracked,
            ],
        )?;
        let parts = parse::diff_parts(&more);
        detail
            .files
            .extend(parse::files(parts.kinds, parts.counts, parts.patches));
    }
    Ok(detail)
}

/// The parent holding a stash's untracked files, when `id` is a stash saved
/// with `--include-untracked`. git makes that a third parent whose subject is
/// `untracked files on <branch>: …`; an octopus merge has a third parent too
/// and no such subject, and is shown as the merge it is.
fn untracked_parent(root: &Path, id: &str) -> Option<String> {
    let third = format!("{id}^3");
    let subject = run(root, &["log", "-1", "--format=%s", &third]).ok()?;
    subject.starts_with("untracked files on ").then_some(third)
}

/// Every branch, local and remote-tracking, and every tag — one
/// `for-each-ref`, with each branch's upstream and how far it is ahead and
/// behind, which `git branch -a` never said.
pub fn refs(root: &Path) -> Result<Refs> {
    let text = run(
        root,
        &[
            "for-each-ref",
            &format!("--format={}", parse::REFS_FORMAT),
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )?;
    Ok(parse::refs(&text))
}

/// Every remote the config names, whether or not anything has been fetched
/// from it — the branches under `refs/remotes/` cannot say that a remote
/// exists until the first fetch or push. Exit 1 is `--get-regexp`'s "nothing
/// matched": a repository with no remotes, not a failure.
pub fn remotes(root: &Path) -> Result<Vec<Remote>> {
    let text = run_allowing(
        root,
        &[
            "config",
            "-z",
            "--get-regexp",
            r"^remote\..*\.(url|pushurl)$",
        ],
        &[0, 1],
    )?;
    Ok(parse::remotes(&text))
}

/// Where the working tree stands: branch, upstream, every changed path, and
/// any operation git has left half done.
pub fn status(root: &Path) -> Result<Status> {
    let text = run(
        root,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "-z",
        ],
    )?;
    let mut status = parse::status(&text);
    status.operation = dirs(root).ok().and_then(|dirs| operation(&dirs.git));
    // git does not descend into another repository, so one shows as a single
    // untracked directory; the ones with no commits are the ones git will
    // refuse to add. A metadata read each, and only for untracked directories.
    for entry in &mut status.entries {
        if entry.untracked && entry.path.ends_with('/') {
            entry.nested = is_empty_repository(&root.join(entry.path.trim_end_matches('/')));
        }
    }
    Ok(status)
}

/// The operation a git directory says is waiting, by the files git leaves
/// there until it is continued or aborted. A rebase first: a rebase that
/// stopped on a conflicting pick leaves `CHERRY_PICK_HEAD` beside its own
/// directory, and "rebasing" is the state the way out belongs to.
fn operation(git_dir: &Path) -> Option<GitOperation> {
    if git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir() {
        Some(GitOperation::Rebase)
    } else if git_dir.join("MERGE_HEAD").is_file() {
        Some(GitOperation::Merge)
    } else if git_dir.join("CHERRY_PICK_HEAD").is_file() {
        Some(GitOperation::CherryPick)
    } else if git_dir.join("REVERT_HEAD").is_file() {
        Some(GitOperation::Revert)
    } else {
        None
    }
}

/// Every stash, newest first.
pub fn stashes(root: &Path) -> Result<Vec<Stash>> {
    let text = run(
        root,
        &[
            "stash",
            "list",
            &format!("--format={}", parse::STASH_FORMAT),
        ],
    )?;
    Ok(parse::stashes(&text))
}

/// One path's difference: the index against HEAD when `staged`, the tree
/// against the index otherwise, and the whole file as added for one the
/// index has never seen.
///
/// `git diff` exits 1 when there *is* a difference, which is the answer
/// wanted, so 1 is not a failure here.
pub fn diff_file(root: &Path, path: &str, staged: bool, untracked: bool) -> Result<String> {
    let args: Vec<&str> = if untracked {
        // `/dev/null` is a name git's own diff understands on every platform,
        // Windows included; it is not a file that has to exist.
        vec!["diff", "--no-index", "--no-color", "--", "/dev/null", path]
    } else if staged {
        vec!["diff", "--cached", "--no-color", "--", path]
    } else {
        vec!["diff", "--no-color", "--", path]
    };
    run_allowing(root, &args, &[0, 1])
}

/// Put paths in the index. Quiet on purpose: instant, reversible, and a dock
/// line per click would bury the commands that matter.
pub fn stage(root: &Path, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    // `--ignore-errors`: one path git cannot index — an empty repository
    // inside this one, a file another program holds open — used to stop the
    // whole add, so "Stage all" staged nothing and named the wrong reason.
    // The rest are staged now, and the failure still comes back naming the
    // path that did not go in.
    let mut args = vec!["add", "--ignore-errors", "--"];
    args.extend(paths.iter().map(String::as_str));
    run(root, &args).map(drop)
}

/// One file's bytes: at `spec` — a hash, `HEAD`, or `:0` for the index —
/// through `git show`, or straight from the working tree when `spec` is
/// `None`. Bytes rather than text, because the caller is showing an image.
pub fn blob(root: &Path, spec: Option<&str>, path: &str) -> Result<Vec<u8>> {
    match spec {
        None => std::fs::read(root.join(path)).map_err(|source| Error::Read {
            path: path.to_string(),
            source,
        }),
        Some(spec) => run_bytes(root, &["show", &format!("{spec}:{path}")], &[0]),
    }
}

/// Take paths back out of the index, leaving the working tree alone.
pub fn unstage(root: &Path, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = vec!["reset", "-q", "--"];
    args.extend(paths.iter().map(String::as_str));
    run(root, &args).map(drop)
}

/// Who commits would be signed as. `git config --get` exits 1 for a key that
/// is not set, which here is an answer rather than a failure.
pub fn identity(root: &Path) -> Result<crate::model::GitIdentity> {
    let get = |key: &str| -> Result<Option<String>> {
        let value = run_allowing(root, &["config", "--get", key], &[0, 1])?;
        let value = value.trim();
        Ok((!value.is_empty()).then(|| value.to_string()))
    };
    Ok(crate::model::GitIdentity {
        name: get("user.name")?,
        email: get("user.email")?,
    })
}

/// Whether `dir` holds a git repository that has never had a commit: a
/// `.git` directory with no branch under `refs/heads` and no `packed-refs`.
///
/// The one test rusty applies before it will remove a `.git` — the wizard
/// after esp-generate, and the Git panel when asked to fold such a
/// directory into the repository around it. A repository with any branch is
/// somebody's history and is never this.
pub fn is_empty_repository(dir: &Path) -> bool {
    let git = dir.join(".git");
    if !git.is_dir() {
        return false;
    }
    let has_branch = git
        .join("refs")
        .join("heads")
        .read_dir()
        .is_ok_and(|mut entries| entries.any(|entry| entry.is_ok()));
    !has_branch && !git.join("packed-refs").exists()
}
