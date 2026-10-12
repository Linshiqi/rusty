//! The working tree, out of `git status --porcelain=v2`.

use crate::model::{ChangeKind, Status, StatusEntry};

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
mod tests {
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
}
