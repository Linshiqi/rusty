//! Running `git` in the project and reading its answers.
//!
//! **One process per question.** Every call runs in the opened project, with
//! the user's own `git`: their config, their credentials, their hooks. And
//! every one costs about sixty milliseconds to *start* on Windows before it
//! reads a byte, so the count is the budget: the log is one process, the
//! refs one, the status one, the stash list one, and a commit opened is one.
//! There used to be a `rev-parse` in front of each of them to ask "is this a
//! repository?" — asked now only when a command has already failed, where
//! the answer decides which refusal to give.
//!
//! `core.quotepath` is off per call so a path with a Chinese character in it
//! arrives as itself rather than as octal escapes, colour is off because this
//! is a machine reading it, and `GIT_OPTIONAL_LOCKS=0` stops a background
//! `git status` from taking `index.lock` to refresh the index — which made a
//! commit typed in a terminal at the same moment fail with "Unable to create
//! index.lock", the panel's read beating the user's write.
//!
//! [`stamp`] runs no `git` at all: it reads the sizes and times of the files
//! git keeps its state in, so the panel can ask "did anything move?" every
//! few seconds for nothing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use crate::graph;
use crate::model::{CommitDetail, GitOperation, GitStamp, History, RefKind, Refs, Stash, Status};
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
    let mut args = vec!["add", "--"];
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

/// Whether `root` is inside a working tree — asked only after a command has
/// failed, to tell "not a repository", the one refusal the panel answers
/// with an Initialize button, from every other failure. Not by reading
/// git's message: that is translated when the user's git is.
fn inside_work_tree(root: &Path) -> bool {
    let mut command = Command::new("git");
    command
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0");
    no_window(&mut command);
    command.output().ok().is_some_and(|out| {
        out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
    })
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
    // asking, and a git that waited on either would hang the panel. And no
    // optional lock: see the module header.
    command
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    no_window(&mut command);
    let output = command.output().map_err(|source| Error::Spawn { source })?;
    let accepted = output.status.code().is_some_and(|code| ok.contains(&code));
    if !accepted {
        if !inside_work_tree(root) {
            return Err(Error::NotARepository {
                path: root.display().to_string(),
            });
        }
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

/// No console window for a child of the GUI on Windows.
fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = command;
}

// ─── the stamp ───────────────────────────────────────────────────────────────

/// Where a repository keeps its state: its own git directory (HEAD, the
/// index, an operation's markers) and the common one shared by every
/// worktree (the refs, the stash). The same directory in an ordinary
/// checkout; two in a linked worktree.
#[derive(Clone, Debug)]
struct Dirs {
    git: PathBuf,
    common: PathBuf,
}

/// Found once per root and kept: the stamp is asked for every few seconds,
/// and a `rev-parse` each time would be the process it exists to avoid.
static DIRS: Mutex<Vec<(PathBuf, Dirs)>> = Mutex::new(Vec::new());

fn dirs(root: &Path) -> Result<Dirs> {
    let cached = DIRS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|(known, _)| known == root)
        .map(|(_, dirs)| dirs.clone());
    if let Some(dirs) = cached {
        return Ok(dirs);
    }
    // `--path-format` is git 2.31; an older git echoes the unknown option
    // back as a line and prints the directories relative, so lines that
    // look like options are skipped and relative ones joined to the root.
    let text = run(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    )?;
    let mut found = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .map(|line| {
            let path = PathBuf::from(line);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        });
    let git = found.next().ok_or_else(|| Error::NotARepository {
        path: root.display().to_string(),
    })?;
    let common = found.next().unwrap_or_else(|| git.clone());
    let dirs = Dirs { git, common };
    let mut cache = DIRS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= 16 {
        cache.remove(0);
    }
    cache.push((root.to_path_buf(), dirs.clone()));
    Ok(dirs)
}

/// The repository's fingerprint — see [`GitStamp`]. Reads metadata only.
pub fn stamp(root: &Path) -> Result<GitStamp> {
    let dirs = dirs(root)?;
    let mut head = Fnv::new();
    head.file(&dirs.git.join("HEAD"));
    head.file(&dirs.git.join("logs").join("HEAD"));
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ] {
        head.file(&dirs.git.join(marker));
    }
    let mut refs = Fnv::new();
    refs.file(&dirs.common.join("packed-refs"));
    let refs_dir = dirs.common.join("refs");
    refs.tree(&refs_dir, &refs_dir.join("stash"));
    let mut index = Fnv::new();
    index.file(&dirs.git.join("index"));
    let mut stash = Fnv::new();
    stash.file(&dirs.common.join("refs").join("stash"));
    stash.file(&dirs.common.join("logs").join("refs").join("stash"));
    Ok(GitStamp {
        head: head.finish(),
        refs: refs.finish(),
        index: index.finish(),
        stash: stash.finish(),
    })
}

/// The largest integer a JavaScript number holds exactly, which is as wide
/// as a stamp can be and still cross the wire — see [`GitStamp`].
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// FNV-1a over names, sizes and modification times.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    /// The hash folded to 53 bits: the top eleven are mixed into the rest
    /// rather than dropped.
    fn finish(&self) -> u64 {
        (self.0 ^ (self.0 >> 53)) & MAX_SAFE_INTEGER
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    /// One file or directory: its name, then its size and time — or that it
    /// is absent, which is a state too (a merge finishing removes a marker).
    fn file(&mut self, path: &Path) {
        self.bytes(path.to_string_lossy().as_bytes());
        match std::fs::metadata(path) {
            Ok(meta) => {
                self.bytes(&meta.len().to_le_bytes());
                let nanos = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_nanos());
                self.bytes(&nanos.to_le_bytes());
            }
            Err(_) => self.bytes(b"absent"),
        }
    }

    /// Every file under `dir`, in name order so the same tree hashes the
    /// same, less `skip`. Names are hashed too, so a ref created or deleted
    /// moves the stamp even where no time does.
    fn tree(&mut self, dir: &Path, skip: &Path) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = read.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            if path == skip {
                continue;
            }
            if path.is_dir() {
                self.tree(&path, skip);
            } else {
                self.file(&path);
            }
        }
    }
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

        let refs = refs(dir.path()).expect("refs");
        let main = refs
            .branches
            .iter()
            .find(|b| b.name == "main")
            .expect("main");
        assert!(main.current);
        assert_eq!(
            main.id, merge.commit.id,
            "a branch names the row it sits on"
        );
        assert!(
            refs.branches
                .iter()
                .any(|b| b.name == "feature" && !b.current)
        );

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

    /// A stash saved with `--include-untracked` keeps the new files in a
    /// third parent. The detail has to reach them — a stash of one new file
    /// opened as "no files changed" — and the history must not draw the
    /// stash's own commits as rows and lanes of the graph.
    #[test]
    fn a_stash_of_an_untracked_file_shows_the_file_and_stays_out_of_the_history() {
        let Some(dir) = repository() else {
            eprintln!("skipping: git is not available on this machine");
            return;
        };
        std::fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\nmore\nagain\n").unwrap();
        let stashed = Command::new("git")
            .args(["stash", "push", "--include-untracked", "-m", "wip"])
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .output()
            .expect("git runs");
        assert!(stashed.status.success(), "{stashed:?}");

        let stashes = stashes(dir.path()).expect("stash list");
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].label, "stash@{0}");

        let detail = commit(dir.path(), &stashes[0].label).expect("the stash opens");
        let mut paths: Vec<&str> = detail.files.iter().map(|f| f.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(
            paths,
            vec!["a.txt", "new.txt"],
            "the tracked change and the untracked file, both",
        );
        let new = detail
            .files
            .iter()
            .find(|f| f.path == "new.txt")
            .expect("the untracked file");
        assert_eq!(new.kind, crate::model::ChangeKind::Added);
        assert!(
            new.patch.contains("+fresh"),
            "with its patch: {}",
            new.patch
        );

        let history = history(dir.path(), None, LIMIT).expect("history");
        assert_eq!(
            history.rows.len(),
            4,
            "the branches' commits and nothing of the stash's"
        );
        assert!(history.rows.iter().all(|row| {
            let summary = row.commit.summary.as_str();
            !summary.starts_with("On main")
                && !summary.starts_with("index on")
                && !summary.starts_with("untracked files on")
        }));
        assert_eq!(history.lanes, 2, "the stash added no lane");
    }

    /// The shape the report came in: a repository with one commit, a new
    /// file in a directory git has never seen, stashed by path from the
    /// Changes view (`stash push --include-untracked -- test/led.rs`). The
    /// stash holds nothing tracked, so the first-parent diff is empty and
    /// only the third parent has the file.
    #[test]
    fn a_stash_of_only_an_untracked_file_in_a_new_directory_lists_that_file() {
        let Some(dir) = tempfile::tempdir().ok() else {
            return;
        };
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "t@x")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "t@x")
                .output()
                .ok()
                .filter(|o| o.status.success())
        };
        if git(&["init", "-q", "-b", "master"]).is_none() {
            eprintln!("skipping: git is not available on this machine");
            return;
        }
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        git(&["add", "Cargo.toml"]).expect("add");
        git(&["commit", "-q", "-m", "init"]).expect("commit");
        std::fs::create_dir_all(dir.path().join("test")).unwrap();
        std::fs::write(dir.path().join("test/led.rs"), "fn led() {}\n").unwrap();
        git(&["stash", "push", "--include-untracked", "--", "test/led.rs"]).expect("stash");

        let detail = commit(dir.path(), "stash@{0}").expect("the stash opens");
        assert_eq!(detail.files.len(), 1, "{:?}", detail.files);
        assert_eq!(detail.files[0].path, "test/led.rs");
        assert_eq!(detail.files[0].kind, crate::model::ChangeKind::Added);
        assert!(detail.files[0].patch.contains("+fn led() {}"));

        let history = history(dir.path(), None, LIMIT).expect("history");
        assert_eq!(
            history.rows.len(),
            1,
            "only `init`, none of the stash's rows"
        );
        assert_eq!(history.lanes, 1);
    }

    /// Run git in `dir` as the tests' author, succeeding or panicking.
    fn git_in(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .output()
            .expect("git runs");
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    /// The bug the full decorations fix: a local branch with a slash in its
    /// name is a branch, in the log's labels and in the refs both.
    #[test]
    fn a_local_branch_with_a_slash_is_a_branch_and_not_a_remote() {
        let Some(dir) = repository() else {
            eprintln!("skipping: git is not available on this machine");
            return;
        };
        // `topic/x`, not `feature/x`: the fixture already has a branch named
        // `feature`, and git keeps refs as files, so `refs/heads/feature`
        // cannot also be a directory.
        git_in(dir.path(), &["branch", "topic/x"]);
        let history = history(dir.path(), None, LIMIT).expect("history");
        let labels: Vec<_> = history.rows[0].commit.refs.clone();
        assert!(
            labels
                .iter()
                .any(|l| l.kind == RefKind::Branch && l.name == "topic/x"),
            "{labels:?}"
        );
        let refs = refs(dir.path()).expect("refs");
        let slashed = refs
            .branches
            .iter()
            .find(|b| b.name == "topic/x")
            .expect("listed");
        assert!(!slashed.remote);
        assert_eq!(slashed.remote_name, None);
    }

    /// The stamp crosses the wire as JSON numbers, which JavaScript reads as
    /// doubles. A part wider than 53 bits was refused by the frontend, and
    /// every probe failed without a word while this crate's tests passed.
    #[test]
    fn every_part_of_the_stamp_fits_in_a_javascript_number() {
        let Some(dir) = repository() else {
            eprintln!("skipping: git is not available on this machine");
            return;
        };
        let stamp = stamp(dir.path()).expect("stamp");
        for part in [stamp.head, stamp.refs, stamp.index, stamp.stash] {
            assert!(part <= MAX_SAFE_INTEGER, "{part} is past 2^53 - 1");
        }
    }

    /// The stamp moves in the part that changed and only there: staging
    /// moves the index, a commit moves HEAD and the refs, a stash moves the
    /// stash. Nothing moves it when nothing happened.
    #[test]
    fn the_stamp_moves_where_the_repository_did() {
        let Some(dir) = repository() else {
            eprintln!("skipping: git is not available on this machine");
            return;
        };
        let root = dir.path();
        let first = stamp(root).expect("stamp");
        assert_eq!(stamp(root).expect("again"), first, "nothing happened");

        std::fs::write(root.join("a.txt"), "changed\n").unwrap();
        git_in(root, &["add", "a.txt"]);
        let staged = stamp(root).expect("staged");
        let stale = staged.stale_since(&first);
        assert!(
            stale.status && !stale.history && !stale.stashes,
            "{stale:?}"
        );

        git_in(root, &["commit", "-q", "-m", "change a"]);
        let committed = stamp(root).expect("committed");
        let stale = committed.stale_since(&staged);
        assert!(stale.history && stale.refs && !stale.stashes, "{stale:?}");

        std::fs::write(root.join("a.txt"), "more\n").unwrap();
        git_in(root, &["stash", "push", "-q", "-m", "wip"]);
        let stashed = stamp(root).expect("stashed");
        assert!(stashed.stale_since(&committed).stashes);
    }

    /// A merge that stops on a conflict is an operation the status names,
    /// and aborting it takes the name away again.
    #[test]
    fn a_conflicted_merge_is_named_until_it_is_aborted() {
        let Some(dir) = repository() else {
            eprintln!("skipping: git is not available on this machine");
            return;
        };
        let root = dir.path();
        git_in(root, &["checkout", "-q", "-b", "other"]);
        std::fs::write(root.join("a.txt"), "theirs\n").unwrap();
        git_in(root, &["commit", "-q", "-am", "theirs"]);
        git_in(root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("a.txt"), "ours\n").unwrap();
        git_in(root, &["commit", "-q", "-am", "ours"]);
        let merged = Command::new("git")
            .args(["merge", "other"])
            .current_dir(root)
            .output()
            .expect("git runs");
        assert!(!merged.status.success(), "the merge conflicts");

        let status = status(root).expect("status");
        assert_eq!(status.operation, Some(GitOperation::Merge));
        assert!(status.entries.iter().any(|e| e.conflicted));

        git_in(root, &["merge", "--abort"]);
        assert_eq!(status_of(root).operation, None);
    }

    fn status_of(root: &Path) -> Status {
        status(root).expect("status")
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
