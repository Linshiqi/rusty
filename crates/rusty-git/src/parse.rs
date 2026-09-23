//! Reading what `git` prints, with the formats chosen so it is unambiguous.
//!
//! Every parser here takes text and returns model types, with the real output
//! pinned in its test. The log is asked for with field and record separators
//! that cannot appear in a commit message (`%x1f` and `%x1e`), because a
//! message with a newline or a tab in it is ordinary and a parser that split
//! on either would tear commits in half.

use std::collections::{BTreeMap, HashMap};

use crate::model::{
    Branch, ChangeKind, Commit, CommitDetail, FileChange, RefKind, RefLabel, Refs, Remote, Stash,
    Status, StatusEntry, Tag,
};

/// The format string [`log`] reads. Hash, parents, author, email, author time,
/// subject, decorations — fields on `\x1f`, records on `\x1e`.
pub const LOG_FORMAT: &str = "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e";

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
pub const DETAIL_FORMAT: &str = "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1f%B%x1e";

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

/// The three readings of one diff, as [`diff_parts`] separates them.
pub struct DiffParts {
    pub kinds: Vec<(String, ChangeKind)>,
    pub counts: Vec<(String, Option<u32>, Option<u32>)>,
    pub patches: Vec<(String, String)>,
}

/// `--raw --numstat -p` output split into its three readings. Everything
/// before the first `diff --git` is raw lines (`:`-prefixed) and numstat
/// lines; everything from it on is the patch, where a line that happens to
/// start with a digit is text and not a count.
pub fn diff_parts(text: &str) -> DiffParts {
    let (head, patch) = if text.starts_with("diff --git ") {
        ("", text)
    } else {
        match text.find("\ndiff --git ") {
            Some(at) => (&text[..at + 1], &text[at + 1..]),
            None => (text, ""),
        }
    };
    let mut kinds = Vec::new();
    let mut counts = Vec::new();
    for line in head.lines() {
        if let Some(raw) = line.strip_prefix(':') {
            kinds.extend(raw_line(raw));
        } else if let Some(count) = numstat_line(line) {
            counts.push(count);
        }
    }
    DiffParts {
        kinds,
        counts,
        patches: split_patch(patch),
    }
}

/// One `--raw` line after its colon: `100644 100644 abc def M\tpath`, or
/// `… R100\told\tnew` for a rename, whose new name is the one kept.
fn raw_line(raw: &str) -> Option<(String, ChangeKind)> {
    let (meta, paths) = raw.split_once('\t')?;
    let status = meta.split_whitespace().last()?;
    let kind = match status.chars().next()? {
        'A' => ChangeKind::Added,
        'M' => ChangeKind::Modified,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        _ => ChangeKind::Other,
    };
    let mut paths = paths.split('\t');
    let first = paths.next()?;
    let path = match kind {
        ChangeKind::Renamed => paths.next().unwrap_or(first),
        _ => first,
    };
    Some((path.to_string(), kind))
}

/// `--numstat`: `added\tremoved\tpath`, `-` for binary. Renames arrive as
/// `old => new` or `dir/{old => new}/file`; the name the file ends up with
/// is what is kept.
pub fn numstat(text: &str) -> Vec<(String, Option<u32>, Option<u32>)> {
    text.lines().filter_map(numstat_line).collect()
}

fn numstat_line(line: &str) -> Option<(String, Option<u32>, Option<u32>)> {
    let mut parts = line.split('\t');
    let added = parts.next()?.trim();
    let removed = parts.next()?.trim();
    let counted = |field: &str| field == "-" || field.chars().all(|c| c.is_ascii_digit());
    if added.is_empty() || !counted(added) || !counted(removed) {
        return None;
    }
    let path = rename_target(parts.next()?);
    Some((path, added.parse().ok(), removed.parse().ok()))
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
/// One pass, one block per `diff --git` header, the path read inside the
/// block: `rename to`, then `+++ b/…`, then `--- a/…` for a deletion (whose
/// `+++` is `/dev/null`), then the header itself for a file with no text
/// lines at all — a binary one. Only the block's header is read for it: a
/// line of content that starts with `+++` is text, not a header, so the
/// search stops at the first `@@`.
///
/// It used to search the *whole* patch again for every `--- a/` line, which
/// is quadratic, and a commit touching a few hundred files paused the panel
/// for exactly that long.
pub fn split_patch(patch: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut block = String::new();
    let mut flush = |block: &mut String| {
        if block.is_empty() {
            return;
        }
        let text = std::mem::take(block);
        if let Some(path) = block_path(&text) {
            files.push((path, text));
        }
    };
    for line in patch.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            flush(&mut block);
        }
        if !block.is_empty() || line.starts_with("diff --git ") {
            block.push_str(line);
        }
    }
    flush(&mut block);
    files
}

/// The path one file's block of a patch is about.
fn block_path(block: &str) -> Option<String> {
    let mut lines = block.lines();
    let header = lines.next()?;
    let mut deleted = None;
    for line in lines {
        if line.starts_with("@@") {
            break;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            return Some(path.trim_end().to_string());
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            if let Some(path) = rest.strip_prefix("b/") {
                return Some(path.trim_end().to_string());
            }
            // `+++ /dev/null`: a deletion, named on its `---` line.
            break;
        }
        if let Some(path) = line.strip_prefix("--- a/") {
            deleted = Some(path.trim_end().to_string());
        }
    }
    deleted.or_else(|| header_path(header))
}

/// `diff --git a/<p> b/<p>`, read where the two names are the same — the
/// only case the header alone can be split unambiguously, since a path may
/// contain ` b/`. A rename's block carries `rename to` instead.
fn header_path(header: &str) -> Option<String> {
    let rest = header.strip_prefix("diff --git ")?;
    let len = rest.len().checked_sub(5)?;
    if len % 2 != 0 {
        return None;
    }
    let half = len / 2;
    let old = rest.get(2..2 + half)?;
    let new = rest.get(5 + half..)?;
    (rest.starts_with("a/") && rest.get(2 + half..5 + half)? == " b/" && old == new)
        .then(|| new.to_string())
}

/// Assemble one commit's files from the three readings. Keyed lookups: the
/// linear search per file this replaced was quadratic in a large commit.
pub fn files(
    kinds: Vec<(String, ChangeKind)>,
    counts: Vec<(String, Option<u32>, Option<u32>)>,
    patches: Vec<(String, String)>,
) -> Vec<FileChange> {
    let counts: HashMap<String, (Option<u32>, Option<u32>)> = counts
        .into_iter()
        .map(|(path, added, removed)| (path, (added, removed)))
        .collect();
    let mut patches: HashMap<String, String> = patches.into_iter().collect();
    kinds
        .into_iter()
        .map(|(path, kind)| {
            let (added, removed) = counts.get(&path).copied().unwrap_or((None, None));
            let patch = patches.remove(&path).unwrap_or_default();
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

/// The remotes out of `git config -z --get-regexp` over the `url` and
/// `pushurl` keys: every entry ends in NUL, and is its key, a newline, then
/// its value — so a URL holding a space, or a path on a drive with one in
/// its name, arrives whole.
///
/// A remote's name may itself hold dots (`my.fork`), so the name is what
/// lies between `remote.` and the trailing `.url` or `.pushurl`, never a
/// split on dots. git fetches from the first of several `url` lines, which
/// is the one kept; a remote with a `pushurl` and no `url` is nothing a
/// fetch can use and is left out rather than shown with a URL it lacks.
pub fn remotes(text: &str) -> Vec<Remote> {
    let mut found: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
    for entry in text.split('\0') {
        let Some((key, value)) = entry.split_once('\n') else {
            continue;
        };
        let Some(rest) = key.strip_prefix("remote.") else {
            continue;
        };
        let (name, push) = if let Some(name) = rest.strip_suffix(".pushurl") {
            (name, true)
        } else if let Some(name) = rest.strip_suffix(".url") {
            (name, false)
        } else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let (url, push_url) = found.entry(name.to_string()).or_default();
        let slot = if push { push_url } else { url };
        if slot.is_none() {
            *slot = Some(value.to_string());
        }
    }
    found
        .into_iter()
        .filter_map(|(name, (url, push_url))| {
            Some(Remote {
                name,
                url: url?,
                push_url,
            })
        })
        .collect()
}

/// The format [`refs`] reads, one ref a line and fields on `\x1f`: the full
/// ref name, `*` when checked out, the upstream's full name, how it tracks
/// it (`ahead 1, behind 2`, or `gone`), the hash, the commit an annotated
/// tag peels to, when it was made, what it points at if it is symbolic, and
/// its subject — last, so nothing after it can be torn by what it contains.
pub const REFS_FORMAT: &str = "%(refname)%1f%(HEAD)%1f%(upstream)%1f%(upstream:track,nobracket)%1f%(objectname)%1f%(*objectname)%1f%(creatordate:unix)%1f%(symref)%1f%(subject)";

/// Branches and tags out of `git for-each-ref --format=REFS_FORMAT
/// refs/heads refs/remotes refs/tags`.
///
/// Local or remote is the ref's namespace, never a slash in its name, and a
/// symbolic ref — `origin/HEAD` — points at a branch rather than being one.
pub fn refs(text: &str) -> Refs {
    let mut out = Refs::default();
    for line in text.lines() {
        let fields: Vec<&str> = line.splitn(9, '\x1f').collect();
        let [
            refname,
            head,
            upstream,
            track,
            id,
            peeled,
            time,
            symref,
            subject,
        ] = fields[..]
        else {
            continue;
        };
        if !symref.trim().is_empty() {
            continue;
        }
        let time = time.trim().parse().unwrap_or(0);
        let id = id.trim().to_string();
        let subject = subject.trim_end().to_string();
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            let (ahead, behind, gone) = tracking(track);
            out.branches.push(Branch {
                name: name.to_string(),
                current: head.trim() == "*",
                remote: false,
                upstream: short_ref(upstream),
                tip: id.chars().take(7).collect(),
                id,
                ahead,
                behind,
                gone,
                time,
                subject,
                remote_name: None,
            });
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            out.branches.push(Branch {
                name: name.to_string(),
                current: false,
                remote: true,
                upstream: None,
                tip: id.chars().take(7).collect(),
                id,
                ahead: 0,
                behind: 0,
                gone: false,
                time,
                subject,
                remote_name: name.split('/').next().map(str::to_string),
            });
        } else if let Some(name) = refname.strip_prefix("refs/tags/") {
            let peeled = peeled.trim();
            out.tags.push(Tag {
                name: name.to_string(),
                id: if peeled.is_empty() {
                    id
                } else {
                    peeled.to_string()
                },
                time,
                subject,
            });
        }
    }
    out.branches
        .sort_by(|a, b| a.remote.cmp(&b.remote).then_with(|| a.name.cmp(&b.name)));
    out.tags
        .sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.name.cmp(&b.name)));
    out
}

/// `%(upstream:track,nobracket)`: empty in step, `ahead 1`, `behind 2`,
/// `ahead 1, behind 2`, or `gone`.
fn tracking(track: &str) -> (u32, u32, bool) {
    let track = track.trim();
    if track == "gone" {
        return (0, 0, true);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in track.split(',').map(str::trim) {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

/// `refs/remotes/origin/main` → `origin/main`; `refs/heads/x` → `x`.
fn short_ref(full: &str) -> Option<String> {
    let full = full.trim();
    let short = full
        .strip_prefix("refs/remotes/")
        .or_else(|| full.strip_prefix("refs/heads/"))
        .unwrap_or(full);
    (!short.is_empty()).then(|| short.to_string())
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
                nested: false,
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
        nested: false,
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
mod remote_tests {
    use super::*;

    /// A real answer's shape: a remote whose name holds a dot, a push URL
    /// set apart from the fetch URL, a path with a space in it, and the
    /// trailing NUL git ends on.
    #[test]
    fn remotes_are_read_whole_with_dotted_names_and_push_urls() {
        let text = concat!(
            "remote.origin.url\nhttps://github.com/you/firmware.git\0",
            "remote.my.fork.url\ngit@github.com:you/fork.git\0",
            "remote.my.fork.pushurl\nE:/Work/My Repos/fork.git\0",
        );
        assert_eq!(
            remotes(text),
            vec![
                Remote {
                    name: "my.fork".into(),
                    url: "git@github.com:you/fork.git".into(),
                    push_url: Some("E:/Work/My Repos/fork.git".into()),
                },
                Remote {
                    name: "origin".into(),
                    url: "https://github.com/you/firmware.git".into(),
                    push_url: None,
                },
            ],
        );
    }

    /// No remotes is an empty answer (git exits 1 with nothing printed); a
    /// second `url` does not replace the first, which is the one git fetches
    /// from; and a remote that only has somewhere to push is left out.
    #[test]
    fn nothing_is_nothing_and_the_first_url_is_the_fetch_url() {
        assert!(remotes("").is_empty());
        let text = concat!(
            "remote.origin.url\nhttps://one.example/r.git\0",
            "remote.origin.url\nhttps://two.example/r.git\0",
            "remote.pushonly.pushurl\nhttps://three.example/r.git\0",
            "core.bare\nfalse\0",
        );
        let read = remotes(text);
        assert_eq!(read.len(), 1, "{read:?}");
        assert_eq!(read[0].url, "https://one.example/r.git");
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
    fn a_header_splits_only_where_both_names_agree() {
        assert_eq!(
            header_path("diff --git a/a b/c.png b/a b/c.png").as_deref(),
            Some("a b/c.png")
        );
        assert_eq!(header_path("diff --git a/x b/y"), None);
        assert_eq!(
            header_path("diff --git a/中文.png b/中文.png").as_deref(),
            Some("中文.png")
        );
    }

    /// A real `for-each-ref` answer: the current branch ahead and behind, a
    /// slashed local branch whose upstream is gone, a remote branch, the
    /// remote's HEAD pointer, an annotated tag peeling to its commit and a
    /// lightweight one.
    #[test]
    fn refs_read_branches_by_namespace_and_tags_peeled() {
        let line = |fields: [&str; 9]| format!("{}\n", fields.join("\x1f"));
        let text = [
            line([
                "refs/heads/main",
                "*",
                "refs/remotes/origin/main",
                "ahead 2, behind 1",
                "aaaaaaaa11",
                "",
                "1756940000",
                "",
                "The tip",
            ]),
            line([
                "refs/heads/feature/x",
                " ",
                "refs/remotes/origin/feature/x",
                "gone",
                "bbbbbbbb22",
                "",
                "1756930000",
                "",
                "Work",
            ]),
            line([
                "refs/remotes/origin/main",
                " ",
                "",
                "",
                "cccccccc33",
                "",
                "1756920000",
                "",
                "Theirs",
            ]),
            line([
                "refs/remotes/origin/HEAD",
                " ",
                "",
                "",
                "cccccccc33",
                "",
                "1756920000",
                "refs/remotes/origin/main",
                "Theirs",
            ]),
            line([
                "refs/tags/v1.0",
                " ",
                "",
                "",
                "tagobject44",
                "dddddddd44",
                "1756910000",
                "",
                "Release one",
            ]),
            line([
                "refs/tags/v0.9",
                " ",
                "",
                "",
                "eeeeeeee55",
                "",
                "1756900000",
                "",
                "An old commit",
            ]),
        ]
        .concat();
        let refs = refs(&text);
        let names: Vec<(&str, bool)> = refs
            .branches
            .iter()
            .map(|b| (b.name.as_str(), b.remote))
            .collect();
        assert_eq!(
            names,
            vec![("feature/x", false), ("main", false), ("origin/main", true)],
            "locals by name, then remotes; origin/HEAD is not a branch"
        );
        let main = &refs.branches[1];
        assert!(main.current);
        assert_eq!((main.ahead, main.behind, main.gone), (2, 1, false));
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert_eq!(main.tip, "aaaaaaa");
        assert!(refs.branches[0].gone);
        assert_eq!(refs.branches[2].remote_name.as_deref(), Some("origin"));
        assert_eq!(refs.branches[2].local_name(), "main");
        assert_eq!(refs.tags[0].name, "v1.0");
        assert_eq!(
            refs.tags[0].id, "dddddddd44",
            "an annotated tag names its commit"
        );
        assert_eq!(refs.tags[1].id, "eeeeeeee55");
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
}
