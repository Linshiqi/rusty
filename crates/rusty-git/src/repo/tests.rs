//! Every query here against a real repository, made for the test.

use std::process::{Command, Output};

use super::stamp::MAX_SAFE_INTEGER;
use super::*;

/// A real repository, made for the test: two commits on `main`, a
/// branch with one more, merged back. Skipped, and said so, where there
/// is no `git` — a CI runner without it teaches people to ignore the
/// suite.
fn repository() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().ok()?;
    git_ok(dir.path(), &["init", "-q", "-b", "main"])?;
    std::fs::write(dir.path().join("a.txt"), "one\n").ok()?;
    git_ok(dir.path(), &["add", "a.txt"])?;
    git_ok(dir.path(), &["commit", "-q", "-m", "first"])?;
    git_ok(dir.path(), &["checkout", "-q", "-b", "feature"])?;
    std::fs::write(dir.path().join("b.txt"), "two\n").ok()?;
    git_ok(dir.path(), &["add", "b.txt"])?;
    git_ok(dir.path(), &["commit", "-q", "-m", "add b"])?;
    git_ok(dir.path(), &["checkout", "-q", "main"])?;
    std::fs::write(dir.path().join("a.txt"), "one\nmore\n").ok()?;
    git_ok(dir.path(), &["commit", "-q", "-am", "grow a"])?;
    git_ok(
        dir.path(),
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    )?;
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
    git_in(
        dir.path(),
        &["stash", "push", "--include-untracked", "-m", "wip"],
    );

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
    if git_ok(dir.path(), &["init", "-q", "-b", "master"]).is_none() {
        eprintln!("skipping: git is not available on this machine");
        return;
    }
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
    git_ok(dir.path(), &["add", "Cargo.toml"]).expect("add");
    git_ok(dir.path(), &["commit", "-q", "-m", "init"]).expect("commit");
    std::fs::create_dir_all(dir.path().join("test")).unwrap();
    std::fs::write(dir.path().join("test/led.rs"), "fn led() {}\n").unwrap();
    git_ok(
        dir.path(),
        &["stash", "push", "--include-untracked", "--", "test/led.rs"],
    )
    .expect("stash");

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

/// `git` in `dir` as the tests' author. The identity rides on every
/// call: a runner has no `user.email`, and a commit, a merge or a stash
/// without one is refused for a reason that has nothing to do with the
/// test.
fn git(dir: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "t@x")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "t@x")
        .output()
}

/// [`git`] that succeeded, or `None` where there is no git to run or it
/// refused — which a fixture turns into a skip rather than a failure.
fn git_ok(dir: &Path, args: &[&str]) -> Option<Output> {
    git(dir, args).ok().filter(|o| o.status.success())
}

/// Run git in `dir` as the tests' author, succeeding or panicking.
fn git_in(dir: &Path, args: &[&str]) {
    let output = git_out(dir, args);
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

/// `git`, with an identity, answering rather than asserting — for the
/// calls that are *expected* to fail, which still need the identity or
/// they fail for the wrong reason.
fn git_out(dir: &Path, args: &[&str]) -> Output {
    git(dir, args).expect("git runs")
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
    for part in [
        stamp.head,
        stamp.refs,
        stamp.index,
        stamp.stash,
        stamp.config,
    ] {
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

/// The report: on a machine with `core.autocrlf=true`, "Stage all" over
/// a file with LF endings and an empty repository inside this one said
/// only the CRLF warning, and staged nothing. The warning is not the
/// failure, and one path git cannot add is not a reason to add none.
#[test]
fn a_stage_that_meets_an_empty_repository_stages_the_rest_and_names_it() {
    let Some(dir) = repository() else {
        eprintln!("skipping: git is not available on this machine");
        return;
    };
    let root = dir.path();
    git_in(root, &["config", "core.autocrlf", "true"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir_all(root.join("firmware/src")).unwrap();
    std::fs::write(root.join("firmware/src/main.rs"), "fn main() {}\n").unwrap();
    let init = git_out(&root.join("firmware"), &["init", "-q", "."]);
    assert_eq!(init.status.code(), Some(0), "{init:?}");

    let listed = status(root).expect("status");
    let nested = listed
        .entries
        .iter()
        .find(|entry| entry.path == "firmware/")
        .expect("git lists the repository as one untracked directory");
    assert!(nested.nested, "{nested:?}");
    assert!(
        listed
            .entries
            .iter()
            .filter(|entry| entry.path != "firmware/")
            .all(|entry| !entry.nested),
        "only the empty repository is marked",
    );

    let failure = stage(root, &[".gitignore".into(), "firmware/".into()])
        .expect_err("git refuses the empty repository");
    let said = failure.to_string();
    assert!(
        said.contains("does not have a commit checked out"),
        "the reason, not the warning in front of it: {said}",
    );
    assert!(!said.contains("LF will be replaced"), "{said}");

    let staged = git_out(root, &["diff", "--cached", "--name-only"]);
    assert!(
        String::from_utf8_lossy(&staged.stdout).contains(".gitignore"),
        "the rest went in: {staged:?}",
    );
}

/// A remote exists the moment it is added — before any fetch has put a
/// branch under `refs/remotes/`, which is exactly when somebody who has
/// just connected a repository to GitHub goes looking for it — and the
/// stamp says the config moved, so a remote added in a terminal is read
/// again without anything else being.
#[test]
fn a_remote_is_listed_before_anything_is_fetched_from_it() {
    let Some(dir) = repository() else {
        eprintln!("skipping: git is not available on this machine");
        return;
    };
    let root = dir.path();
    assert!(
        remotes(root).expect("no remotes is an answer").is_empty(),
        "git exits 1 when nothing matches, and that is not a failure",
    );
    let before = stamp(root).expect("stamp");

    let added = git_out(
        root,
        &[
            "remote",
            "add",
            "--",
            "origin",
            "https://github.com/you/firmware.git",
        ],
    );
    assert_eq!(added.status.code(), Some(0), "{added:?}");
    let listed = remotes(root).expect("remotes");
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].name, "origin");
    assert_eq!(listed[0].url, "https://github.com/you/firmware.git");
    assert!(
        refs(root)
            .expect("refs")
            .branches
            .iter()
            .all(|branch| !branch.remote),
        "nothing is fetched yet, which is the case this read exists for",
    );

    let stale = stamp(root).expect("stamp").stale_since(&before);
    assert!(
        stale.remotes && !stale.history && !stale.refs && !stale.status,
        "{stale:?}",
    );
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
    // Through the same door as every other call here, because it needs
    // the identity: a runner has no `user.email`, and `git merge` asks
    // for one *before* it merges — "Committer identity unknown", exit
    // 128, no `MERGE_HEAD`, nothing merged. This passed on any desk with
    // a global git config and failed on every runner for weeks.
    let merged = git_out(root, &["merge", "other"]);
    // And on the conflict specifically, not on the failure. `!success`
    // was true for a merge that never happened, so the one assertion
    // standing between this test and its own environment said nothing:
    // git exits 1 for a conflict and 128 for a refusal.
    assert_eq!(
        merged.status.code(),
        Some(1),
        "the merge conflicts: {}{}",
        String::from_utf8_lossy(&merged.stdout),
        String::from_utf8_lossy(&merged.stderr),
    );

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
