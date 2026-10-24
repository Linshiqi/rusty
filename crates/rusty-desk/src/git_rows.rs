//! The Git panel's file lists as rows: a heading per group — the files in
//! conflict, the staged ones, the rest — each folding away under its
//! heading, and inside each either a flat list or the files' folders as a
//! tree, VS Code's *View as Tree*, a folder that holds nothing but one
//! other folder drawn as one row (`book/src`), as VS Code compacts them.
//!
//! Pure, so which rows a collapse hides and how a tree compacts are tests;
//! `app/git_view.rs` turns a row into what the panel draws.

use std::collections::{BTreeMap, HashSet};

use rusty_git::StatusEntry;

/// A list of the panel's, in the order it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    /// Unmerged paths: what a stopped merge, rebase, cherry-pick or revert
    /// is waiting on. VS Code's *Merge Changes*.
    Conflicts,
    Staged,
    /// Changed in the working tree, and untracked.
    Changes,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::Conflicts, Group::Staged, Group::Changes];

    /// The number the panel and the window speak of a group by.
    pub fn index(self) -> i32 {
        match self {
            Group::Conflicts => 0,
            Group::Staged => 1,
            Group::Changes => 2,
        }
    }

    pub fn from_index(index: i32) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.index() == index)
    }

    /// The name `workbench.toml` keeps a collapsed group under.
    pub fn name(self) -> &'static str {
        match self {
            Group::Conflicts => "conflicts",
            Group::Staged => "staged",
            Group::Changes => "changes",
        }
    }

    pub fn from_name(name: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.name() == name)
    }

    /// Whether `entry` is in this group. An unmerged path is in the
    /// conflict group alone, whatever else its index says.
    pub fn holds(self, entry: &StatusEntry) -> bool {
        match self {
            Group::Conflicts => entry.is_conflicted(),
            Group::Staged => !entry.is_conflicted() && entry.staged.is_some(),
            Group::Changes => {
                !entry.is_conflicted() && (entry.unstaged.is_some() || entry.untracked)
            }
        }
    }

    /// The heading is drawn even when the group is empty: the working
    /// tree's list always, so a clean tree still says where changes go.
    fn always_shown(self) -> bool {
        self == Group::Changes
    }
}

/// What is folded away: whole groups, and folders within a group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folded {
    pub groups: HashSet<Group>,
    pub folders: HashSet<(Group, String)>,
}

/// One row of the panel's lists.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Heading {
        group: Group,
        count: usize,
        folded: bool,
    },
    /// A folder of the tree: its path as the tree knows it, what the row
    /// says (more than one segment where folders were compacted), how deep
    /// it sits, how many files are under it and whether it is folded.
    Folder {
        group: Group,
        path: String,
        label: String,
        depth: usize,
        count: usize,
        folded: bool,
    },
    /// A file: which entry it is and how deep it sits — 0 in a flat list.
    File {
        group: Group,
        entry: usize,
        depth: usize,
    },
}

/// The rows for `entries`: each group's heading, then, unless the group is
/// folded, its files — in porcelain's order as a flat list, or under their
/// folders, folders first and each level in name order, as a tree.
pub fn rows(entries: &[StatusEntry], tree: bool, folded: &Folded) -> Vec<Row> {
    let mut out = Vec::new();
    for group in Group::ALL {
        let members: Vec<usize> = (0..entries.len())
            .filter(|&i| group.holds(&entries[i]))
            .collect();
        if members.is_empty() && !group.always_shown() {
            continue;
        }
        let group_folded = folded.groups.contains(&group);
        out.push(Row::Heading {
            group,
            count: members.len(),
            folded: group_folded,
        });
        if group_folded {
            continue;
        }
        if tree {
            let root = build(entries, &members);
            flatten(&root, group, "", 0, folded, entries, &mut out);
        } else {
            out.extend(members.into_iter().map(|entry| Row::File {
                group,
                entry,
                depth: 0,
            }));
        }
    }
    out
}

/// Every path of `group` under the folder `dir` — for a folder row's verbs.
pub fn paths_under(entries: &[StatusEntry], group: Group, dir: &str) -> Vec<String> {
    let prefix = format!("{}/", dir.trim_end_matches('/'));
    entries
        .iter()
        .filter(|e| group.holds(e) && e.path.starts_with(&prefix))
        .map(|e| e.path.clone())
        .collect()
}

/// Every path in `group`.
pub fn paths_in(entries: &[StatusEntry], group: Group) -> Vec<String> {
    entries
        .iter()
        .filter(|e| group.holds(e))
        .map(|e| e.path.clone())
        .collect()
}

/// A folder of the tree being built: its subfolders by name and the
/// entries directly in it.
#[derive(Default)]
struct Node {
    folders: BTreeMap<String, Node>,
    files: Vec<usize>,
}

impl Node {
    fn count(&self) -> usize {
        self.files.len() + self.folders.values().map(Node::count).sum::<usize>()
    }
}

/// The tree of `members`' paths. An untracked directory's entry ends in
/// `/` (git reports a repository of its own that way) and is a file of its
/// parent here, as it is a row of its own in the flat list.
fn build(entries: &[StatusEntry], members: &[usize]) -> Node {
    let mut root = Node::default();
    for &index in members {
        let path = entries[index].path.trim_end_matches('/');
        let mut parts: Vec<&str> = path.split('/').collect();
        parts.pop();
        let mut node = &mut root;
        for part in parts {
            node = node.folders.entry(part.to_string()).or_default();
        }
        node.files.push(index);
    }
    root
}

fn flatten(
    node: &Node,
    group: Group,
    at: &str,
    depth: usize,
    folded: &Folded,
    entries: &[StatusEntry],
    out: &mut Vec<Row>,
) {
    for (name, child) in &node.folders {
        // A folder holding nothing but one folder is drawn as one row.
        let mut label = name.clone();
        let mut child = child;
        while child.files.is_empty() && child.folders.len() == 1 {
            let (next, grandchild) = child.folders.iter().next().expect("one folder");
            label = format!("{label}/{next}");
            child = grandchild;
        }
        let path = if at.is_empty() {
            label.clone()
        } else {
            format!("{at}/{label}")
        };
        let is_folded = folded.folders.contains(&(group, path.clone()));
        out.push(Row::Folder {
            group,
            path: path.clone(),
            label,
            depth,
            count: child.count(),
            folded: is_folded,
        });
        if !is_folded {
            flatten(child, group, &path, depth + 1, folded, entries, out);
        }
    }
    let mut files = node.files.clone();
    files.sort_by(|a, b| entries[*a].path.cmp(&entries[*b].path));
    out.extend(files.into_iter().map(|entry| Row::File {
        group,
        entry,
        depth,
    }));
}

#[cfg(test)]
mod tests {
    use rusty_git::{ChangeKind, ConflictKind};

    use super::*;

    fn entry(path: &str, staged: bool, changed: bool) -> StatusEntry {
        StatusEntry {
            path: path.into(),
            staged: staged.then_some(ChangeKind::Modified),
            unstaged: changed.then_some(ChangeKind::Modified),
            untracked: false,
            conflict: None,
            nested: false,
        }
    }

    fn conflicted(path: &str) -> StatusEntry {
        StatusEntry {
            conflict: Some(ConflictKind::BothModified),
            ..entry(path, false, false)
        }
    }

    fn said(rows: &[Row], entries: &[StatusEntry]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Heading {
                    group,
                    count,
                    folded,
                } => format!(
                    "{}{} {count}",
                    if *folded { "+" } else { "-" },
                    group.name()
                ),
                Row::Folder {
                    label,
                    depth,
                    count,
                    folded,
                    ..
                } => format!(
                    "{}{}{label} {count}",
                    "  ".repeat(*depth),
                    if *folded { "+" } else { "-" }
                ),
                Row::File { entry, depth, .. } => {
                    format!("{}{}", "  ".repeat(*depth), entries[*entry].path)
                }
            })
            .collect()
    }

    /// An unmerged path is in the conflict list and in no other, whatever
    /// its index says; a path both staged and changed is in both lists.
    #[test]
    fn each_entry_is_in_the_groups_it_belongs_to() {
        let entries = vec![
            conflicted("a.rs"),
            entry("b.rs", true, true),
            entry("c.rs", false, true),
        ];
        let rows = rows(&entries, false, &Folded::default());
        assert_eq!(
            said(&rows, &entries),
            [
                "-conflicts 1",
                "a.rs",
                "-staged 1",
                "b.rs",
                "-changes 2",
                "b.rs",
                "c.rs"
            ]
        );
    }

    /// The changes heading stands over a clean tree; an empty conflict or
    /// staged list has none.
    #[test]
    fn only_the_changes_heading_stands_over_nothing() {
        let rows = rows(&[], false, &Folded::default());
        assert_eq!(said(&rows, &[]), ["-changes 0"]);
    }

    /// A folded group keeps its heading and its count, and draws nothing
    /// under it.
    #[test]
    fn a_folded_group_keeps_its_heading_and_hides_its_files() {
        let entries = vec![entry("b.rs", true, false), entry("c.rs", false, true)];
        let folded = Folded {
            groups: [Group::Staged].into(),
            ..Folded::default()
        };
        let rows = rows(&entries, false, &folded);
        assert_eq!(said(&rows, &entries), ["+staged 1", "-changes 1", "c.rs"]);
    }

    /// The tree: folders first, each level in name order, a chain of
    /// folders holding one folder each drawn as one row, every folder
    /// counting the files under it.
    #[test]
    fn a_tree_compacts_single_folders_and_counts_what_is_under_each() {
        let entries = vec![
            entry("README.md", false, true),
            entry("book/src/01.md", false, true),
            entry("book/src/02.md", false, true),
            entry("book/src/fig/a.svg", false, true),
            entry("firmware/src/main.rs", false, true),
            entry("book/custom.css", false, true),
        ];
        let rows = rows(&entries, true, &Folded::default());
        assert_eq!(
            said(&rows, &entries),
            [
                "-changes 6",
                "-book 4",
                "  -src 3",
                "    -fig 1",
                "      book/src/fig/a.svg",
                "    book/src/01.md",
                "    book/src/02.md",
                "  book/custom.css",
                "-firmware/src 1",
                "  firmware/src/main.rs",
                "README.md",
            ]
        );
    }

    /// A folded folder keeps its row and hides what is under it, in its own
    /// group only.
    #[test]
    fn a_folded_folder_hides_what_is_under_it_in_its_group() {
        let entries = vec![
            entry("book/src/01.md", true, true),
            entry("book/src/02.md", false, true),
        ];
        let folded = Folded {
            folders: [(Group::Changes, "book/src".to_string())].into(),
            ..Folded::default()
        };
        let rows = rows(&entries, true, &folded);
        assert_eq!(
            said(&rows, &entries),
            [
                "-staged 1",
                "-book/src 1",
                "  book/src/01.md",
                "-changes 2",
                "+book/src 2",
            ]
        );
    }

    /// A folder's verbs reach the paths under it in its own group, and a
    /// folder whose name begins another's is not that folder.
    #[test]
    fn a_folder_holds_the_paths_under_it_and_no_neighbour_of_its_name() {
        let entries = vec![
            entry("src/a.rs", false, true),
            entry("src2/b.rs", false, true),
            entry("src/c.rs", true, false),
        ];
        assert_eq!(paths_under(&entries, Group::Changes, "src"), ["src/a.rs"]);
        assert_eq!(paths_in(&entries, Group::Staged), ["src/c.rs"]);
    }
}
