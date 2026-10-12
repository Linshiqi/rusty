//! One commit's diff as `git show --raw --numstat -p` prints it, taken
//! apart into each file's kind, counts and patch.

use std::collections::HashMap;

use crate::model::{ChangeKind, FileChange};

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
///
/// A whole answer at once is what the tests read; [`diff_parts`] reads the
/// lines one at a time, among the raw lines.
#[cfg(test)]
fn numstat(text: &str) -> Vec<(String, Option<u32>, Option<u32>)> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
