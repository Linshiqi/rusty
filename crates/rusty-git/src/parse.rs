//! Reading what `git` prints, with the formats chosen so it is unambiguous.
//!
//! Every parser here takes text and returns model types, with the real output
//! pinned in its test. The log is asked for with field and record separators
//! that cannot appear in a commit message (`%x1f` and `%x1e`), because a
//! message with a newline or a tab in it is ordinary and a parser that split
//! on either would tear commits in half.

use crate::model::{
    Branch, ChangeKind, Commit, FileChange, RefKind, RefLabel, Stash, Status, StatusEntry,
};

/// The format string [`log`] reads. Hash, parents, author, email, author time,
/// subject, decorations — fields on `\x1f`, records on `\x1e`.
pub const LOG_FORMAT: &str = "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e";

/// Commits out of `git log --format=LOG_FORMAT`.
pub fn log(text: &str) -> Vec<Commit> {
    text.split('\x1e')
        .map(str::trim)
        .filter(|record| !record.is_empty())
        .filter_map(commit)
        .collect()
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

/// `%D`: `HEAD -> main, origin/main, tag: v0.4.0`.
///
/// A remote-tracking branch is told from a local one by the slash, because
/// that is all the format carries. A local branch named `feature/x` will be
/// drawn as remote; the alternative is a second `git remote` round trip per
/// log, and the label is still the right text.
pub fn decorations(text: &str) -> Vec<RefLabel> {
    let mut labels = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(name) = part.strip_prefix("HEAD -> ") {
            labels.push(RefLabel {
                kind: RefKind::Head,
                name: name.to_string(),
            });
        } else if part == "HEAD" {
            labels.push(RefLabel {
                kind: RefKind::Head,
                name: String::new(),
            });
        } else if let Some(name) = part.strip_prefix("tag: ") {
            labels.push(RefLabel {
                kind: RefKind::Tag,
                name: name.to_string(),
            });
        } else if part.contains('/') {
            labels.push(RefLabel {
                kind: RefKind::Remote,
                name: part.to_string(),
            });
        } else {
            labels.push(RefLabel {
                kind: RefKind::Branch,
                name: part.to_string(),
            });
        }
    }
    labels
}

/// `git show --name-status --format=`: one `M\tpath` line per file, renames
/// as `R100\told\tnew`.
pub fn name_status(text: &str) -> Vec<(String, ChangeKind)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let status = parts.next()?.trim();
            let first = parts.next()?;
            let kind = match status.chars().next()? {
                'A' => ChangeKind::Added,
                'M' => ChangeKind::Modified,
                'D' => ChangeKind::Deleted,
                'R' => ChangeKind::Renamed,
                _ => ChangeKind::Other,
            };
            // A rename names both; the file the commit leaves behind is the
            // new one.
            let path = match kind {
                ChangeKind::Renamed => parts.next().unwrap_or(first),
                _ => first,
            };
            Some((path.to_string(), kind))
        })
        .collect()
}

/// `git show --numstat --format=`: `added\tremoved\tpath`, `-` for binary.
/// Renames arrive as `old => new` or `dir/{old => new}/file`; the name the
/// file ends up with is what is kept.
pub fn numstat(text: &str) -> Vec<(String, Option<u32>, Option<u32>)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let added = parts.next()?.trim().parse().ok();
            let removed = parts.next()?.trim().parse().ok();
            let path = rename_target(parts.next()?);
            Some((path, added, removed))
        })
        .collect()
}

/// `dir/{old => new}/file` → `dir/new/file`; `old => new` → `new`.
fn rename_target(spelling: &str) -> String {
    if let (Some(open), Some(close)) = (spelling.find('{'), spelling.find('}'))
        && open < close
        && let Some((_, new)) = spelling[open + 1..close].split_once(" => ")
    {
        return format!("{}{}{}", &spelling[..open], new, &spelling[close + 1..]);
    }
    if let Some((_, new)) = spelling.split_once(" => ") {
        return new.to_string();
    }
    spelling.to_string()
}

/// A whole-commit patch split per file, keyed by the file's new path.
///
/// Splits on `diff --git` headers and reads the path off `+++ b/…`, falling
/// back to `--- a/…` for a deletion (whose `+++` is `/dev/null`).
pub fn split_patch(patch: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut current: Option<(Option<String>, String)> = None;
    for line in patch.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            if let Some((Some(path), text)) = current.take() {
                files.push((path, text));
            }
            current = Some((None, String::new()));
        }
        if let Some((path, text)) = current.as_mut() {
            if path.is_none() {
                if let Some(rest) = line.strip_prefix("+++ b/") {
                    *path = Some(rest.trim_end().to_string());
                } else if let Some(rest) = line.strip_prefix("--- a/")
                    && !patch_has_plus_for(rest, patch)
                {
                    *path = Some(rest.trim_end().to_string());
                }
            }
            text.push_str(line);
        }
    }
    if let Some((Some(path), text)) = current {
        files.push((path, text));
    }
    files
}

/// Whether a `+++ b/<path>` line exists anywhere — the cheap way to know a
/// `--- a/` line is a deletion's rather than a modification's first half.
fn patch_has_plus_for(path: &str, patch: &str) -> bool {
    let needle = format!("+++ b/{}", path.trim_end());
    patch.lines().any(|l| l.trim_end() == needle)
}

/// Assemble one commit's files from the three answers.
pub fn files(
    statuses: Vec<(String, ChangeKind)>,
    stats: Vec<(String, Option<u32>, Option<u32>)>,
    patches: Vec<(String, String)>,
) -> Vec<FileChange> {
    statuses
        .into_iter()
        .map(|(path, kind)| {
            let (added, removed) = stats
                .iter()
                .find(|(p, _, _)| *p == path)
                .map(|(_, a, r)| (*a, *r))
                .unwrap_or((None, None));
            let patch = patches
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, text)| text.clone())
                .unwrap_or_default();
            FileChange {
                path,
                kind,
                added,
                removed,
                patch,
            }
        })
        .collect()
}

/// The format [`branches`] reads: short name, `*` when checked out, the
/// upstream's short name, the tip's short hash — on tabs.
pub const BRANCH_FORMAT: &str =
    "%(refname:short)%09%(HEAD)%09%(upstream:short)%09%(objectname:short)";

/// Branches out of `git branch -a --format=BRANCH_FORMAT`.
///
/// `origin/HEAD` is a pointer at another remote branch, not a branch anyone
/// checks out, and is dropped.
pub fn branches(text: &str) -> Vec<Branch> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.trim().to_string();
            if name.is_empty() || name.ends_with("/HEAD") {
                return None;
            }
            let current = parts.next()?.trim() == "*";
            let upstream = parts
                .next()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_string);
            let tip = parts.next().unwrap_or("").trim().to_string();
            Some(Branch {
                remote: name.contains('/'),
                name,
                current,
                upstream,
                tip,
            })
        })
        .collect()
}

/// The format [`stashes`] reads: the ref (`stash@{0}`), the subject, the
/// time — on the log's separators.
pub const STASH_FORMAT: &str = "%gd%x1f%s%x1f%at%x1e";

/// Stashes out of `git stash list --format=STASH_FORMAT`, newest first.
pub fn stashes(text: &str) -> Vec<Stash> {
    text.split('\x1e')
        .map(str::trim)
        .filter(|record| !record.is_empty())
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

/// `git status --porcelain=v2 --branch --untracked-files=all -z`.
///
/// Version 2 with `-z`, and nothing less: v1 has no branch block, and without
/// `-z` a path with a space or a quote comes back quoted in C style, which
/// is a second parser to get wrong. Every entry is one NUL-terminated token;
/// a rename's original path is the token after it.
pub fn status(text: &str) -> Status {
    let mut status = Status::default();
    let mut tokens = text.split('\0').filter(|t| !t.is_empty());
    while let Some(token) = tokens.next() {
        if let Some(header) = token.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" => {
                    if value == "(detached)" {
                        status.detached = true;
                    } else {
                        status.head = Some(value.to_string());
                    }
                }
                "branch.upstream" => status.upstream = Some(value.to_string()),
                "branch.ab" => {
                    for part in value.split_whitespace() {
                        if let Some(n) = part.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        // The path is the last field and may itself contain spaces, so the
        // fixed fields before it are counted rather than split off the end.
        let entry = match token.chars().next() {
            Some('1') => token.splitn(9, ' ').nth(8).map(|path| changed(token, path)),
            Some('2') => {
                // `2 XY sub mH mI mW hH hI Xscore path` and then the original
                // path as its own token, which is consumed and not shown: the
                // file the tree has is the new one.
                let _original = tokens.next();
                token
                    .splitn(10, ' ')
                    .nth(9)
                    .map(|path| changed(token, path))
            }
            Some('u') => token.splitn(11, ' ').nth(10).map(|path| StatusEntry {
                conflicted: true,
                ..changed(token, path)
            }),
            Some('?') => token.strip_prefix("? ").map(|path| StatusEntry {
                path: path.to_string(),
                staged: None,
                unstaged: Some(ChangeKind::Added),
                untracked: true,
                conflicted: false,
            }),
            // `!` is ignored files, asked for by nobody here.
            _ => None,
        };
        if let Some(entry) = entry {
            status.entries.push(entry);
        }
    }
    status
}

/// An ordinary or renamed entry: the `XY` field is the second one.
fn changed(token: &str, path: &str) -> StatusEntry {
    let mut xy = token.split(' ').nth(1).unwrap_or("..").chars();
    let staged = kind_of(xy.next().unwrap_or('.'));
    let unstaged = kind_of(xy.next().unwrap_or('.'));
    StatusEntry {
        path: path.to_string(),
        staged,
        unstaged,
        untracked: false,
        conflicted: false,
    }
}

/// One letter of porcelain's `XY`.
fn kind_of(code: char) -> Option<ChangeKind> {
    match code {
        '.' => None,
        'A' => Some(ChangeKind::Added),
        'M' | 'T' => Some(ChangeKind::Modified),
        'D' => Some(ChangeKind::Deleted),
        'R' | 'C' => Some(ChangeKind::Renamed),
        _ => Some(ChangeKind::Other),
    }
}

#[cfg(test)]
mod working_tree_tests {
    use super::*;

    /// A real `--porcelain=v2 --branch -z` answer: the branch block, a file
    /// changed in the tree only, one staged as new, a staged rename with its
    /// original path as the next token, a path with a space in it, and an
    /// untracked file.
    #[test]
    fn status_reads_the_branch_block_and_every_kind_of_entry() {
        let text = concat!(
            "# branch.oid 20d12f8de4db7a9000627bf3c1d8ca9ecc8500db\0",
            "# branch.head master\0",
            "# branch.upstream origin/master\0",
            "# branch.ab +2 -1\0",
            "1 .M N... 100644 100644 100644 e1a9ba7 e1a9ba7 README.md\0",
            "1 A. N... 000000 100644 100644 0000000 8a9ff8a src/new file.rs\0",
            "2 R. N... 100644 100644 100644 1111111 1111111 R100 src/after.rs\0src/before.rs\0",
            "? notes.txt\0",
        );
        let status = status(text);
        assert_eq!(status.head.as_deref(), Some("master"));
        assert!(!status.detached);
        assert_eq!(status.upstream.as_deref(), Some("origin/master"));
        assert_eq!((status.ahead, status.behind), (2, 1));

        let paths: Vec<&str> = status.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["README.md", "src/new file.rs", "src/after.rs", "notes.txt"],
            "a rename shows its new name and swallows the original's token",
        );
        assert_eq!(status.entries[0].staged, None);
        assert_eq!(status.entries[0].unstaged, Some(ChangeKind::Modified));
        assert_eq!(status.entries[1].staged, Some(ChangeKind::Added));
        assert_eq!(status.entries[1].unstaged, None);
        assert_eq!(status.entries[2].staged, Some(ChangeKind::Renamed));
        assert!(status.entries[3].untracked);
        assert_eq!(status.entries[3].unstaged, Some(ChangeKind::Added));
    }

    #[test]
    fn a_detached_head_and_a_conflict_are_said() {
        let text = concat!(
            "# branch.oid abc\0",
            "# branch.head (detached)\0",
            "u UU N... 100644 100644 100644 100644 a b c src/lib.rs\0",
        );
        let status = status(text);
        assert!(status.detached);
        assert_eq!(status.head, None);
        assert!(status.entries[0].conflicted);
        assert_eq!(status.entries[0].path, "src/lib.rs");
    }

    #[test]
    fn a_clean_tree_is_a_branch_block_and_nothing_else() {
        let status = status("# branch.oid abc\0# branch.head main\0");
        assert_eq!(status.head.as_deref(), Some("main"));
        assert!(status.entries.is_empty());
        assert_eq!(status.upstream, None, "no upstream block, no upstream");
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn name_status_reads_renames_as_their_new_name() {
        let listed =
            name_status("M\tsrc/lib.rs\nA\tsrc/new.rs\nD\told.rs\nR100\tfrom.rs\tto.rs\nT\tlink\n");
        assert_eq!(
            listed,
            vec![
                ("src/lib.rs".to_string(), ChangeKind::Modified),
                ("src/new.rs".to_string(), ChangeKind::Added),
                ("old.rs".to_string(), ChangeKind::Deleted),
                ("to.rs".to_string(), ChangeKind::Renamed),
                ("link".to_string(), ChangeKind::Other),
            ]
        );
    }

    #[test]
    fn numstat_reads_counts_binaries_and_rename_spellings() {
        let listed = numstat(
            "12\t3\tsrc/lib.rs\n-\t-\tlogo.png\n1\t1\tsrc/{old => new}/mod.rs\n0\t2\ta.rs => b.rs\n",
        );
        assert_eq!(listed[0], ("src/lib.rs".to_string(), Some(12), Some(3)));
        assert_eq!(listed[1], ("logo.png".to_string(), None, None));
        assert_eq!(listed[2].0, "src/new/mod.rs");
        assert_eq!(listed[3].0, "b.rs");
    }

    /// Two files, one of them deleted — whose `+++` is `/dev/null`, so the
    /// path comes off the `---` line instead.
    #[test]
    fn a_patch_splits_per_file_and_names_a_deletion_by_its_old_path() {
        let patch = concat!(
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "index 1..2 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n",
            "@@ -1 +1 @@\n-old\n+new\n",
            "diff --git a/gone.rs b/gone.rs\n",
            "deleted file mode 100644\nindex 3..0\n--- a/gone.rs\n+++ /dev/null\n",
            "@@ -1 +0,0 @@\n-bye\n",
        );
        let files = split_patch(patch);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "src/lib.rs");
        assert!(files[0].1.starts_with("diff --git a/src/lib.rs"));
        assert!(files[0].1.contains("+new\n"));
        assert_eq!(files[1].0, "gone.rs");
        assert!(files[1].1.contains("-bye\n"));
    }

    #[test]
    fn branches_mark_the_current_one_and_drop_the_remote_head_pointer() {
        let text = "master\t*\torigin/master\t20d12f8\nfeature\t\t\tabc1234\norigin/HEAD\t\t\t20d12f8\norigin/master\t\t\t20d12f8\n";
        let listed = branches(text);
        assert_eq!(listed.len(), 3, "origin/HEAD is not a branch");
        assert!(listed[0].current);
        assert_eq!(listed[0].upstream.as_deref(), Some("origin/master"));
        assert!(!listed[1].current);
        assert_eq!(listed[1].upstream, None);
        assert!(listed[2].remote);
        assert!(!listed[0].remote);
    }
}
