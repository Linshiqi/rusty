//! The log, a commit opened and the stash list, each asked for with its
//! records ended by `%x1e` and its fields apart on `%x1f`.

use crate::model::{Commit, CommitDetail, RefKind, RefLabel, Stash};

use super::diff::{diff_parts, files};

/// A commit's first seven fields — hash, parents, author, email, author
/// time, subject, decorations, on `%x1f` — which [`LOG_FORMAT`] and
/// [`DETAIL_FORMAT`] both begin with. `commit` reads them by position, so
/// the two must agree on them, and a macro is what `concat!` can build a
/// constant from.
macro_rules! commit_fields {
    () => {
        "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D"
    };
}

/// The format string [`log`] reads. Hash, parents, author, email, author time,
/// subject, decorations — fields on `\x1f`, records on `\x1e`.
pub const LOG_FORMAT: &str = concat!(commit_fields!(), "%x1e");

/// Commits out of `git log --format=LOG_FORMAT`.
pub fn log(text: &str) -> Vec<Commit> {
    records(text).filter_map(commit).collect()
}

/// The records of an answer asked for with `%x1e` after each: trimmed, and
/// with the empty ones — the tail after the last separator — dropped.
fn records(text: &str) -> impl Iterator<Item = &str> {
    text.split('\x1e')
        .map(str::trim)
        .filter(|record| !record.is_empty())
}

fn commit(record: &str) -> Option<Commit> {
    let mut fields = record.split('\x1f');
    let id = fields.next()?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let parents = fields
        .next()?
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let author = fields.next()?.to_string();
    let email = fields.next()?.to_string();
    let time = fields.next()?.trim().parse().unwrap_or(0);
    let summary = fields.next()?.to_string();
    let refs = decorations(fields.next().unwrap_or(""));
    Some(Commit {
        short: id.chars().take(7).collect(),
        id,
        parents,
        author,
        email,
        time,
        summary,
        refs,
    })
}

/// `%D`, asked for with `--decorate=full`: `HEAD -> refs/heads/main,
/// refs/remotes/origin/main, tag: refs/tags/v0.4.0`.
///
/// Full names, because the short form tells a remote-tracking branch from a
/// local one only by a slash — and `feature/x` is a local branch. It was
/// drawn as a remote, and the panel then refused to check it out or delete
/// it. `origin/HEAD` is a pointer rather than a branch and `refs/stash` has
/// a view of its own, so neither is a label. The short spelling is still
/// read, the old way, for a caller that did not ask for full names.
pub fn decorations(text: &str) -> Vec<RefLabel> {
    let mut labels = Vec::new();
    let label = |kind: RefKind, name: &str| RefLabel {
        kind,
        name: name.to_string(),
    };
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(name) = part.strip_prefix("HEAD -> ") {
            let name = name.strip_prefix("refs/heads/").unwrap_or(name);
            labels.push(label(RefKind::Head, name));
        } else if part == "HEAD" {
            labels.push(label(RefKind::Head, ""));
        } else if let Some(name) = part.strip_prefix("tag: ") {
            labels.push(label(
                RefKind::Tag,
                name.strip_prefix("refs/tags/").unwrap_or(name),
            ));
        } else if let Some(name) = part.strip_prefix("refs/heads/") {
            labels.push(label(RefKind::Branch, name));
        } else if let Some(name) = part.strip_prefix("refs/remotes/") {
            if !name.ends_with("/HEAD") {
                labels.push(label(RefKind::Remote, name));
            }
        } else if part.starts_with("refs/") {
            // `refs/stash`, notes, anything else a tool keeps under refs/.
        } else if part.contains('/') {
            labels.push(label(RefKind::Remote, part));
        } else {
            labels.push(label(RefKind::Branch, part));
        }
    }
    labels
}

/// The format [`detail`] reads: the log's fields, then the whole message.
pub const DETAIL_FORMAT: &str = concat!(commit_fields!(), "%x1f%B%x1e");

/// One commit opened, out of one `git show --format=DETAIL_FORMAT --raw
/// --numstat -p`: the record and its message up to the record separator,
/// then three readings of the diff — `--raw` for each file's kind,
/// `--numstat` for its counts, the patch for its text.
///
/// One process where there were seven. Every `git` costs about sixty
/// milliseconds to start on Windows before it does anything, and opening a
/// commit ran seven of them one after another.
pub fn detail(text: &str) -> Option<CommitDetail> {
    let (record, rest) = text.split_once('\x1e')?;
    let record = record.trim_start_matches('\n');
    let commit = commit(record)?;
    let body = record
        .splitn(8, '\x1f')
        .nth(7)
        .unwrap_or("")
        .trim_end()
        .to_string();
    let parts = diff_parts(rest);
    Some(CommitDetail {
        commit,
        body,
        files: files(parts.kinds, parts.counts, parts.patches),
    })
}

/// The format [`stashes`] reads: the ref (`stash@{0}`), the subject, the
/// time — on the log's separators.
pub const STASH_FORMAT: &str = "%gd%x1f%s%x1f%at%x1e";

/// Stashes out of `git stash list --format=STASH_FORMAT`, newest first.
pub fn stashes(text: &str) -> Vec<Stash> {
    records(text)
        .filter_map(|record| {
            let mut fields = record.split('\x1f');
            let label = fields.next()?.trim().to_string();
            let index = label
                .strip_prefix("stash@{")?
                .strip_suffix('}')?
                .parse()
                .ok()?;
            let message = fields.next()?.to_string();
            let time = fields.next()?.trim().parse().unwrap_or(0);
            Some(Stash {
                index,
                label,
                message,
                time,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::model::ChangeKind;

    use super::*;

    /// `commit` reads the fields by position and `detail` finds the message
    /// as the eighth, so the strings git is handed are pinned byte for byte.
    #[test]
    fn the_log_and_detail_formats_are_the_strings_git_is_handed() {
        assert_eq!(LOG_FORMAT, "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e");
        assert_eq!(
            DETAIL_FORMAT,
            "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1f%B%x1e"
        );
    }

    /// Two real records, with the separators git emits for the format, and a
    /// subject carrying a tab and a comma to prove neither splits anything.
    #[test]
    fn the_log_splits_on_its_own_separators_and_nothing_else() {
        let text = concat!(
            "20d12f8de4db7a9000627bf3c1d8ca9ecc8500db\x1f59ea8cd0\x1fcs3\x1fcs3@cersign.com\x1f",
            "1756940000\x1fv0.4.0: tests get a lens,\tand Windows gets a debugger\x1f",
            "HEAD -> master, tag: v0.4.0, origin/master\x1e\n",
            "59ea8cd000000000000000000000000000000000\x1f\x1fcs3\x1fcs3@cersign.com\x1f1756930000\x1f",
            "CodeLLDB is a download\x1f\x1e\n",
        );
        let commits = log(text);
        assert_eq!(commits.len(), 2);
        let first = &commits[0];
        assert_eq!(first.short, "20d12f8");
        assert_eq!(first.parents, vec!["59ea8cd0"]);
        assert_eq!(first.time, 1_756_940_000);
        assert_eq!(
            first.summary,
            "v0.4.0: tests get a lens,\tand Windows gets a debugger"
        );
        assert_eq!(
            first.refs,
            vec![
                RefLabel {
                    kind: RefKind::Head,
                    name: "master".into()
                },
                RefLabel {
                    kind: RefKind::Tag,
                    name: "v0.4.0".into()
                },
                RefLabel {
                    kind: RefKind::Remote,
                    name: "origin/master".into()
                },
            ]
        );
        assert!(commits[1].parents.is_empty(), "a root has no parents");
        assert!(commits[1].refs.is_empty());
    }

    #[test]
    fn a_detached_head_is_a_head_with_no_branch() {
        assert_eq!(
            decorations("HEAD, tag: v1"),
            vec![
                RefLabel {
                    kind: RefKind::Head,
                    name: String::new()
                },
                RefLabel {
                    kind: RefKind::Tag,
                    name: "v1".into()
                },
            ]
        );
    }

    /// Full names: a local branch with a slash is a branch, `origin/HEAD` is
    /// no label, the stash is none either, and a tag keeps its short name.
    #[test]
    fn full_decorations_tell_a_slashed_local_branch_from_a_remote() {
        let labels = decorations(
            "HEAD -> refs/heads/main, refs/heads/feature/x, refs/remotes/origin/main, \
             refs/remotes/origin/HEAD, tag: refs/tags/v1.0, refs/stash",
        );
        assert_eq!(
            labels,
            vec![
                RefLabel {
                    kind: RefKind::Head,
                    name: "main".into()
                },
                RefLabel {
                    kind: RefKind::Branch,
                    name: "feature/x".into()
                },
                RefLabel {
                    kind: RefKind::Remote,
                    name: "origin/main".into()
                },
                RefLabel {
                    kind: RefKind::Tag,
                    name: "v1.0".into()
                },
            ]
        );
    }

    /// One `git show --raw --numstat -p` answer, as git 2.52 wrote it: the
    /// record with a two-paragraph message, then raw lines for a change, an
    /// addition, a rename and a binary file, their counts, and the patch —
    /// whose content lines start with digits and `+++` without being read as
    /// counts or headers.
    #[test]
    fn one_show_carries_the_record_the_message_and_every_file() {
        let text = concat!(
            "20d12f8de4db7a9000627bf3c1d8ca9ecc8500db\x1f59ea8cd0\x1fcs3\x1fcs3@x\x1f",
            "1756940000\x1fThe subject\x1fHEAD -> refs/heads/main\x1f",
            "The subject\n\nThe body, with a tab\there.\n\x1e\n\n",
            ":100644 100644 aaaaaaa bbbbbbb M\tsrc/lib.rs\n",
            ":000000 100644 0000000 ccccccc A\tsrc/new file.rs\n",
            ":100644 100644 ddddddd eeeeeee R087\told/name.rs\tnew/name.rs\n",
            ":100644 100644 fffffff 1111111 M\tlogo.png\n",
            "2\t1\tsrc/lib.rs\n",
            "1\t0\tsrc/new file.rs\n",
            "3\t3\t{old => new}/name.rs\n",
            "-\t-\tlogo.png\n",
            "\n",
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "index aaaaaaa..bbbbbbb 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n",
            "@@ -1,2 +1,3 @@\n 12\n-3\t4\tnot a count\n+++ not a header\n+5\n",
            "diff --git a/src/new file.rs b/src/new file.rs\n",
            "new file mode 100644\nindex 0000000..ccccccc\n--- /dev/null\n+++ b/src/new file.rs\n",
            "@@ -0,0 +1 @@\n+fn new() {}\n",
            "diff --git a/old/name.rs b/new/name.rs\n",
            "similarity index 87%\nrename from old/name.rs\nrename to new/name.rs\n",
            "--- a/old/name.rs\n+++ b/new/name.rs\n@@ -1 +1 @@\n-a\n+b\n",
            "diff --git a/logo.png b/logo.png\n",
            "index fffffff..1111111 100644\nBinary files a/logo.png and b/logo.png differ\n",
        );
        let detail = detail(text).expect("parses");
        assert_eq!(detail.commit.summary, "The subject");
        assert_eq!(detail.body, "The subject\n\nThe body, with a tab\there.");
        assert_eq!(detail.commit.refs[0].name, "main");
        let summary: Vec<(&str, ChangeKind, Option<u32>, Option<u32>)> = detail
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.kind, f.added, f.removed))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("src/lib.rs", ChangeKind::Modified, Some(2), Some(1)),
                ("src/new file.rs", ChangeKind::Added, Some(1), Some(0)),
                ("new/name.rs", ChangeKind::Renamed, Some(3), Some(3)),
                ("logo.png", ChangeKind::Modified, None, None),
            ]
        );
        assert!(detail.files[0].patch.contains("+++ not a header\n"));
        assert!(detail.files[1].patch.contains("+fn new() {}"));
        assert!(detail.files[2].patch.contains("rename to new/name.rs"));
        assert!(
            detail.files[3].patch.contains("Binary files"),
            "a binary file is named off its header"
        );
    }

    #[test]
    fn stashes_carry_their_index_and_note() {
        let text = "stash@{0}\x1fOn master: half a feature\x1f1756940000\x1e\nstash@{1}\x1fWIP on master: 20d12f8 v0.4.0\x1f1756930000\x1e\n";
        let listed = stashes(text);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].index, 0);
        assert_eq!(listed[0].label, "stash@{0}");
        assert_eq!(listed[0].message, "On master: half a feature");
        assert_eq!(listed[1].index, 1);
        assert_eq!(listed[1].time, 1_756_930_000);
    }
}
