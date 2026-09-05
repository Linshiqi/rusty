//! Running `git` in the project and reading its answers.
//!
//! Three questions, three invocations each of at most a few commands. Every
//! call runs in the opened project, with the user's own `git`: their config,
//! their credentials, their hooks. `core.quotepath` is turned off per call so
//! a path with a Chinese character in it arrives as itself rather than as
//! octal escapes, and colour is off because this is a machine reading it.

use std::path::Path;
use std::process::Command;

use crate::graph;
use crate::model::{Branch, CommitDetail, History, Stash, Status};
use crate::parse;

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
    ensure_repository(root)?;
    let mut args = vec![
        "log".to_string(),
        "--topo-order".to_string(),
        format!("--format={}", parse::LOG_FORMAT),
        // One more than asked, so "there are older commits" is a fact seen
        // rather than inferred from hitting the limit exactly.
        format!("-n{}", limit + 1),
    ];
    match rev {
        Some(rev) => args.push(rev.to_string()),
        None => args.push("--all".to_string()),
    }
    let text = run(root, &args)?;
    let mut commits = parse::log(&text);
    let truncated = commits.len() > limit;
    commits.truncate(limit);
    let mut laid_out = graph::lay_out(commits);
    laid_out.truncated = truncated;
    laid_out.head = head(root);
    Ok(laid_out)
}

/// What HEAD names: a branch, or the short hash when detached.
fn head(root: &Path) -> Option<String> {
    let name = run(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    name.or_else(|| {
        run(root, &["rev-parse", "--short", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
    })
}

/// One commit opened: message, files, and each file's patch.
pub fn commit(root: &Path, id: &str) -> Result<CommitDetail> {
    ensure_repository(root)?;
    let record = run(
        root,
        &[
            "log",
            "-1",
            "--topo-order",
            &format!("--format={}", parse::LOG_FORMAT),
            id,
        ],
    )
    .map_err(|_| Error::NoSuchCommit { id: id.to_string() })?;
    let commit = parse::log(&record)
        .into_iter()
        .next()
        .ok_or_else(|| Error::NoSuchCommit { id: id.to_string() })?;
    let body = run(root, &["show", "-s", "--format=%B", id])?
        .trim_end()
        .to_string();
    // `-m --first-parent` so a merge shows what it brought in against its
    // first parent, the way Fork does, rather than the empty combined diff
    // `git show` prints for a clean merge.
    let statuses = parse::name_status(&run(
        root,
        &[
            "show",
            "--name-status",
            "--format=",
            "-m",
            "--first-parent",
            id,
        ],
    )?);
    let stats = parse::numstat(&run(
        root,
        &["show", "--numstat", "--format=", "-m", "--first-parent", id],
    )?);
    let patches = parse::split_patch(&run(
        root,
        &[
            "show",
            "--format=",
            "-p",
            "-m",
            "--first-parent",
            "--no-color",
            id,
        ],
    )?);
    Ok(CommitDetail {
        commit,
        body,
        files: parse::files(statuses, stats, patches),
    })
}

/// Every branch, local and remote-tracking, current one marked.
pub fn branches(root: &Path) -> Result<Vec<Branch>> {
    ensure_repository(root)?;
    let text = run(
        root,
        &[
            "branch",
            "-a",
            &format!("--format={}", parse::BRANCH_FORMAT),
        ],
    )?;
    Ok(parse::branches(&text))
}

/// Where the working tree stands: branch, upstream, and every changed path.
pub fn status(root: &Path) -> Result<Status> {
    ensure_repository(root)?;
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
    Ok(parse::status(&text))
}

/// Every stash, newest first.
pub fn stashes(root: &Path) -> Result<Vec<Stash>> {
    ensure_repository(root)?;
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
    ensure_repository(root)?;
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
    let mut args = vec!["add", "--"];
    args.extend(paths.iter().map(String::as_str));
    run(root, &args).map(drop)
}

/// One file's bytes: at `spec` — a hash, `HEAD`, or `:0` for the index —
/// through `git show`, or straight from the working tree when `spec` is
/// `None`. Bytes rather than text, because the caller is showing an image.
pub fn blob(root: &Path, spec: Option<&str>, path: &str) -> Result<Vec<u8>> {
    ensure_repository(root)?;
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

/// A directory with no repository above it gets a plain answer, not a
/// `git log` error about a missing ref.
fn ensure_repository(root: &Path) -> Result<()> {
    match run(root, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(out) if out.trim() == "true" => Ok(()),
        _ => Err(Error::NotARepository {
            path: root.display().to_string(),
        }),
    }
}

/// One `git` invocation, its stdout as text.
fn run<S: AsRef<str>>(root: &Path, args: &[S]) -> Result<String> {
    run_allowing(root, args, &[0])
}

/// [`run`], treating any of `ok` as success — for the commands whose exit
/// code is an answer rather than a verdict.
fn run_allowing<S: AsRef<str>>(root: &Path, args: &[S], ok: &[i32]) -> Result<String> {
    run_bytes(root, args, ok).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The invocation itself, stdout as bytes — what [`blob`] needs, and what
/// every text reader lossily decodes.
fn run_bytes<S: AsRef<str>>(root: &Path, args: &[S], ok: &[i32]) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .args(["-c", "core.quotepath=off", "-c", "color.ui=never"])
        .args(args.iter().map(AsRef::as_ref))
        .current_dir(root)
        .env_remove("RUSTUP_TOOLCHAIN");
    // The pager and the editor must never be consulted: this is a machine
    // asking, and a git that waited on either would hang the panel.
    command
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = command.output().map_err(|source| Error::Spawn { source })?;
    let accepted = output.status.code().is_some_and(|code| ok.contains(&code));
    if !accepted {
        return Err(Error::Git {
            command: args
                .iter()
                .map(AsRef::as_ref)
                .next()
                .unwrap_or("")
                .to_string(),
            detail: String::from_utf8_lossy(&output.stderr)
                .trim()
                .lines()
                .next()
                .unwrap_or("no message")
                .to_string(),
        });
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real repository, made for the test: two commits on `main`, a
    /// branch with one more, merged back. Skipped, and said so, where there
    /// is no `git` — a CI runner without it teaches people to ignore the
    /// suite.
    fn repository() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().ok()?;
        let git = |args: &[&str]| {
            let mut command = Command::new("git");
            command.args(args).current_dir(dir.path());
            command
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "t@x")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "t@x");
            command.output().ok().filter(|o| o.status.success())
        };
        git(&["init", "-q", "-b", "main"])?;
        std::fs::write(dir.path().join("a.txt"), "one\n").ok()?;
        git(&["add", "a.txt"])?;
        git(&["commit", "-q", "-m", "first"])?;
        git(&["checkout", "-q", "-b", "feature"])?;
        std::fs::write(dir.path().join("b.txt"), "two\n").ok()?;
        git(&["add", "b.txt"])?;
        git(&["commit", "-q", "-m", "add b"])?;
        git(&["checkout", "-q", "main"])?;
        std::fs::write(dir.path().join("a.txt"), "one\nmore\n").ok()?;
        git(&["commit", "-q", "-am", "grow a"])?;
        git(&["merge", "-q", "--no-ff", "-m", "merge feature", "feature"])?;
        Some(dir)
    }

    #[test]
    fn the_history_of_a_real_repository_is_laid_out_with_its_merge() {
        let Some(dir) = repository() else {
            eprintln!("skipping: git is not available on this machine");
            return;
        };
        let history = history(dir.path(), None, LIMIT).expect("history");
        assert_eq!(history.rows.len(), 4);
        assert!(!history.truncated);
        assert_eq!(history.head.as_deref(), Some("main"));
        let merge = &history.rows[0];
        assert_eq!(merge.commit.summary, "merge feature");
        assert_eq!(merge.commit.parents.len(), 2);
        assert!(
            merge
                .commit
                .refs
                .iter()
                .any(|r| r.kind == crate::model::RefKind::Head && r.name == "main")
        );
        assert_eq!(history.lanes, 2, "a merged branch is a second lane");

        let branches = branches(dir.path()).expect("branches");
        let main = branches.iter().find(|b| b.name == "main").expect("main");
        assert!(main.current);
        assert!(branches.iter().any(|b| b.name == "feature" && !b.current));

        let detail = commit(dir.path(), &merge.commit.id).expect("the merge");
        assert_eq!(detail.body, "merge feature");
        assert_eq!(
            detail
                .files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["b.txt"],
            "against its first parent, the merge brought in b.txt",
        );
        assert!(detail.files[0].patch.contains("+two"));
    }

    #[test]
    fn a_directory_without_a_repository_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("plain");
        std::fs::create_dir_all(&outside).unwrap();
        // Only meaningful with a git to ask; without one the spawn error is
        // the honest answer and this test has nothing to pin.
        match history(&outside, None, LIMIT) {
            Err(Error::NotARepository { .. }) | Err(Error::Spawn { .. }) => {}
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}
