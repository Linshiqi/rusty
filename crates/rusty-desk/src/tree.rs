//! The project tree's arithmetic, apart from the window (`app/tree.rs`):
//! the box a name is typed in, placed among the rows where the entry will
//! be; where an arrow key moves the selection; and what a name typed in
//! the box is refused for. Pure and tested; the window draws what it says.
//!
//! The refusals of a name are `rusty_edit`'s own (`name_problem`, the rule
//! `rename_entry` keeps) plus the one only the tree can answer while the
//! name is typed — that something of that name is already there — read
//! off the entries the tree holds rather than the disk, so judging a key
//! never touches the disk on the thread that draws.

use rusty_edit::{Entry, NameProblem};

use crate::project::ShownRow;

/// The box a name is typed in, and what it is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Naming {
    /// A new file, or a new folder, in `dir` (`""` is the root).
    New { dir: String, folder: bool },
    /// The entry at `path` renamed where it stands.
    Rename { path: String, dir: bool },
}

impl Naming {
    /// The directory the name is typed for.
    pub fn dir(&self) -> &str {
        match self {
            Naming::New { dir, .. } => dir,
            Naming::Rename { path, .. } => rusty_edit::parent_dir(path),
        }
    }
}

/// A row as the tree draws it: an entry, or the box a name is typed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Entry(ShownRow),
    /// The box, as deep as the entry it names, and whether that is a
    /// folder.
    Name {
        depth: usize,
        folder: bool,
    },
}

/// The rows with the box among them, where the entry it names will be:
/// a new folder first in its folder, a new file after the folders and
/// before the first file — among its future siblings, as VS Code puts it —
/// and a rename in the entry's own row. The folder a new entry goes in is
/// open, its caller having opened it; one that is not on screen gets no
/// box.
pub fn with_box(rows: Vec<ShownRow>, naming: Option<&Naming>) -> Vec<Shown> {
    let place = match naming {
        Some(Naming::New { dir, folder }) => {
            let (start, depth) = if dir.is_empty() {
                (0, 0)
            } else {
                match rows.iter().position(|r| &r.path == dir && r.open) {
                    Some(at) => (at + 1, rows[at].depth + 1),
                    None => return rows.into_iter().map(Shown::Entry).collect(),
                }
            };
            let mut at = start;
            if !folder {
                while at < rows.len() && rows[at].depth >= depth {
                    if rows[at].depth == depth && !rows[at].dir {
                        break;
                    }
                    at += 1;
                }
            }
            Some((at, depth, *folder, false))
        }
        Some(Naming::Rename { path, .. }) => rows
            .iter()
            .position(|r| &r.path == path)
            .map(|at| (at, rows[at].depth, rows[at].dir, true)),
        None => None,
    };
    let mut shown: Vec<Shown> = rows.into_iter().map(Shown::Entry).collect();
    if let Some((at, depth, folder, replace)) = place {
        let name = Shown::Name { depth, folder };
        if replace {
            shown[at] = name;
        } else {
            shown.insert(at, name);
        }
    }
    shown
}

/// A key that moves through the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
}

/// What a key does to the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Select(String),
    /// Open a closed folder.
    Expand(String),
    /// Close an open folder.
    Collapse(String),
    Stay,
}

/// What `arrow` does with `selected` on the rows on screen — VS Code's
/// list: Up and Down step a row; Right opens a closed folder and steps
/// into an open one; Left closes an open folder and otherwise goes to the
/// folder the entry is in. A selection not on screen (inside a closed
/// folder, or nothing yet) starts from the top or the bottom.
pub fn arrow(rows: &[ShownRow], selected: Option<&str>, arrow: Arrow) -> Step {
    let select = |at: usize| {
        rows.get(at)
            .map_or(Step::Stay, |row| Step::Select(row.path.clone()))
    };
    let last = rows.len().saturating_sub(1);
    let Some(at) = selected.and_then(|path| rows.iter().position(|r| r.path == path)) else {
        return match arrow {
            Arrow::Up | Arrow::End => select(last),
            _ => select(0),
        };
    };
    let row = &rows[at];
    match arrow {
        Arrow::Down => select(at + 1),
        Arrow::Up if at > 0 => select(at - 1),
        Arrow::Up => Step::Stay,
        Arrow::Home => select(0),
        Arrow::End => select(last),
        Arrow::Right if row.dir && !row.open => Step::Expand(row.path.clone()),
        Arrow::Right if row.dir => match rows.get(at + 1) {
            Some(next) if next.depth > row.depth => Step::Select(next.path.clone()),
            _ => Step::Stay,
        },
        Arrow::Right => Step::Stay,
        Arrow::Left if row.dir && row.open => Step::Collapse(row.path.clone()),
        Arrow::Left => rows[..at]
            .iter()
            .rev()
            .find(|r| r.depth + 1 == row.depth)
            .map_or(Step::Stay, |parent| Step::Select(parent.path.clone())),
    }
}

/// Why the name typed in the box cannot be taken, said under it as it is
/// typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Nothing typed.
    Empty,
    /// Something of that name is already where it would go.
    Taken(String),
    /// A separator in a rename: a rename is a name, and a path is a move.
    Separator,
    /// A new entry's path that starts or ends with a separator, or has two
    /// together.
    Edge,
    /// `.` or `..`.
    Dots(String),
    /// What this system does not take for a name.
    Invalid(String),
    /// A folder on the way to a new entry is a file.
    Blocked(String),
}

/// What is wrong with `typed` for `naming`, or `None` when the entry can
/// be made. A rename is one name; a new entry may be a path — `src/x.rs`
/// makes the folder on the way, as VS Code's does — and every segment of
/// it is held to the name rule. `fold` compares names without case, as a
/// Windows or macOS disk does.
pub fn refusal(naming: &Naming, typed: &str, entries: &[Entry], fold: bool) -> Option<Refusal> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Some(Refusal::Empty);
    }
    let segment = |name: &str| match rusty_edit::name_problem(name) {
        None => None,
        Some(NameProblem::Empty) => Some(Refusal::Edge),
        Some(NameProblem::Separator) => Some(Refusal::Separator),
        Some(NameProblem::Dots) => Some(Refusal::Dots(name.to_string())),
        Some(NameProblem::Invalid) => Some(Refusal::Invalid(name.to_string())),
    };
    match naming {
        Naming::Rename { path, .. } => {
            if let Some(problem) = segment(typed) {
                return Some(problem);
            }
            let to = joined(naming.dir(), typed);
            let same = if fold {
                to.to_lowercase() == path.to_lowercase()
            } else {
                &to == path
            };
            (!same && find(entries, &to, fold).is_some()).then(|| Refusal::Taken(typed.to_string()))
        }
        Naming::New { dir, .. } => {
            let parts: Vec<&str> = typed.split(['/', '\\']).collect();
            let mut at = dir.clone();
            for (index, part) in parts.iter().enumerate() {
                if part.is_empty() {
                    return Some(Refusal::Edge);
                }
                if let Some(problem) = segment(part) {
                    return Some(problem);
                }
                at = joined(&at, part);
                let last = index + 1 == parts.len();
                match find(entries, &at, fold) {
                    Some(_) if last => return Some(Refusal::Taken(typed.to_string())),
                    Some(false) => return Some(Refusal::Blocked(part.to_string())),
                    _ => {}
                }
            }
            None
        }
    }
}

/// The relative path a name typed for `naming` makes: separators as the
/// tree writes them, space trimmed.
pub fn target(naming: &Naming, typed: &str) -> String {
    let typed = typed.trim().replace('\\', "/");
    joined(naming.dir(), &typed)
}

/// Whether the entry at `path` is there, and whether it is a folder.
fn find(entries: &[Entry], path: &str, fold: bool) -> Option<bool> {
    let mut level = entries;
    let mut found = None;
    for part in path.split('/') {
        let entry = level.iter().find(|e| {
            if fold {
                e.name.to_lowercase() == part.to_lowercase()
            } else {
                e.name == part
            }
        })?;
        found = Some(entry.is_dir);
        level = &entry.children;
    }
    found
}

fn joined(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// How many bytes of a name a rename selects to begin with: the name
/// before its extension, as VS Code selects it, so typing replaces the
/// name and keeps `.rs`. A folder's whole name; a dotfile's whole name.
pub fn stem_bytes(name: &str, folder: bool) -> usize {
    match name.rfind('.') {
        Some(at) if at > 0 && !folder => at,
        _ => name.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, depth: usize, dir: bool, open: bool) -> ShownRow {
        ShownRow {
            depth,
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            dir,
            open,
        }
    }

    /// `src` open with `bin` closed and two files in it, `tests` closed,
    /// two files at the root.
    fn rows() -> Vec<ShownRow> {
        vec![
            row("src", 0, true, true),
            row("src/bin", 1, true, false),
            row("src/lib.rs", 1, false, false),
            row("src/main.rs", 1, false, false),
            row("tests", 0, true, false),
            row("Cargo.toml", 0, false, false),
            row("README.md", 0, false, false),
        ]
    }

    fn drawn(shown: &[Shown]) -> Vec<String> {
        shown
            .iter()
            .map(|s| match s {
                Shown::Entry(r) => format!("{}{}", "  ".repeat(r.depth), r.name),
                Shown::Name { depth, folder } => {
                    format!(
                        "{}[{}]",
                        "  ".repeat(*depth),
                        if *folder { "dir" } else { "file" }
                    )
                }
            })
            .collect()
    }

    #[test]
    fn a_new_entry_is_named_where_it_will_be() {
        let new = |dir: &str, folder| Naming::New {
            dir: dir.into(),
            folder,
        };
        // A file in `src`: after `bin`, before the first file.
        assert_eq!(
            drawn(&with_box(rows(), Some(&new("src", false))))[1..4],
            ["  bin", "  [file]", "  lib.rs"]
        );
        // A folder in `src`: first.
        assert_eq!(
            drawn(&with_box(rows(), Some(&new("src", true))))[..2],
            ["src", "  [dir]"]
        );
        // At the root: a file after the last folder, past what is open in it.
        assert_eq!(
            drawn(&with_box(rows(), Some(&new("", false))))[4..7],
            ["tests", "[file]", "Cargo.toml"]
        );
        assert_eq!(drawn(&with_box(rows(), Some(&new("", true))))[0], "[dir]");
        // A folder that only has folders: the file goes last in it.
        let only_dirs = vec![
            row("a", 0, true, true),
            row("a/b", 1, true, false),
            row("z.rs", 0, false, false),
        ];
        assert_eq!(
            drawn(&with_box(only_dirs, Some(&new("a", false)))),
            ["a", "  b", "  [file]", "z.rs"]
        );
        // A closed folder is not on screen to name into: no box.
        assert_eq!(
            with_box(rows(), Some(&new("tests", false))).len(),
            rows().len()
        );
    }

    #[test]
    fn a_rename_is_typed_in_the_entrys_own_row() {
        let shown = with_box(
            rows(),
            Some(&Naming::Rename {
                path: "src/lib.rs".into(),
                dir: false,
            }),
        );
        assert_eq!(shown.len(), rows().len());
        assert_eq!(drawn(&shown)[2], "  [file]");
    }

    #[test]
    fn the_arrows_walk_the_rows_as_vs_codes_list_does() {
        let rows = rows();
        let sel = |p: &str| Step::Select(p.to_string());
        assert_eq!(arrow(&rows, None, Arrow::Down), sel("src"));
        assert_eq!(arrow(&rows, None, Arrow::Up), sel("README.md"));
        assert_eq!(arrow(&rows, Some("src"), Arrow::Down), sel("src/bin"));
        assert_eq!(arrow(&rows, Some("src"), Arrow::Up), Step::Stay);
        assert_eq!(arrow(&rows, Some("README.md"), Arrow::Down), Step::Stay);
        // Right: open a closed folder, step into an open one, nothing on a file.
        assert_eq!(
            arrow(&rows, Some("tests"), Arrow::Right),
            Step::Expand("tests".into())
        );
        assert_eq!(arrow(&rows, Some("src"), Arrow::Right), sel("src/bin"));
        assert_eq!(arrow(&rows, Some("src/lib.rs"), Arrow::Right), Step::Stay);
        // Left: close an open folder, otherwise up to the folder it is in.
        assert_eq!(
            arrow(&rows, Some("src"), Arrow::Left),
            Step::Collapse("src".into())
        );
        assert_eq!(arrow(&rows, Some("src/main.rs"), Arrow::Left), sel("src"));
        assert_eq!(arrow(&rows, Some("src/bin"), Arrow::Left), sel("src"));
        assert_eq!(arrow(&rows, Some("Cargo.toml"), Arrow::Left), Step::Stay);
        // A selection hidden in a closed folder starts from the top.
        assert_eq!(arrow(&rows, Some("tests/a.rs"), Arrow::Down), sel("src"));
        assert_eq!(
            arrow(&rows, Some("src/lib.rs"), Arrow::End),
            sel("README.md")
        );
        assert_eq!(arrow(&rows, Some("src/lib.rs"), Arrow::Home), sel("src"));
        assert_eq!(arrow(&[], None, Arrow::Down), Step::Stay);
    }

    fn entry(path: &str, children: Vec<Entry>) -> Entry {
        Entry {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            is_dir: !path.contains('.'),
            children,
        }
    }

    fn entries() -> Vec<Entry> {
        vec![
            entry(
                "src",
                vec![
                    entry("src/main.rs", vec![]),
                    entry("src/驱动", vec![entry("src/驱动/mod.rs", vec![])]),
                ],
            ),
            entry("Cargo.toml", vec![]),
        ]
    }

    #[test]
    fn a_name_typed_is_refused_for_what_it_is() {
        let new_file = Naming::New {
            dir: "src".into(),
            folder: false,
        };
        let rename = Naming::Rename {
            path: "src/main.rs".into(),
            dir: false,
        };
        let e = entries();
        assert_eq!(refusal(&new_file, "  ", &e, true), Some(Refusal::Empty));
        assert_eq!(refusal(&new_file, "lib.rs", &e, true), None);
        assert_eq!(
            refusal(&new_file, "MAIN.rs", &e, true),
            Some(Refusal::Taken("MAIN.rs".into()))
        );
        assert_eq!(
            refusal(&new_file, "MAIN.rs", &e, false),
            None,
            "a disk with case"
        );
        // A path makes its folders, through ones that are there.
        assert_eq!(refusal(&new_file, "驱动/spi.rs", &e, true), None);
        assert_eq!(refusal(&new_file, "a/b/c.rs", &e, true), None);
        assert_eq!(
            refusal(&new_file, "main.rs/x.rs", &e, true),
            Some(Refusal::Blocked("main.rs".into()))
        );
        assert_eq!(refusal(&new_file, "a//b.rs", &e, true), Some(Refusal::Edge));
        assert_eq!(refusal(&new_file, "/b.rs", &e, true), Some(Refusal::Edge));
        assert_eq!(
            refusal(&new_file, "a/../b.rs", &e, true),
            Some(Refusal::Dots("..".into()))
        );
        // A rename is a name: no separators; its own name, or another case
        // of it, is no clash.
        assert_eq!(refusal(&rename, "lib.rs", &e, true), None);
        assert_eq!(refusal(&rename, "main.rs", &e, true), None);
        assert_eq!(refusal(&rename, "Main.rs", &e, true), None);
        assert_eq!(
            refusal(&rename, "驱动", &e, true),
            Some(Refusal::Taken("驱动".into()))
        );
        assert_eq!(
            refusal(&rename, "x/y.rs", &e, true),
            Some(Refusal::Separator)
        );
        assert_eq!(
            refusal(&rename, "..", &e, true),
            Some(Refusal::Dots("..".into()))
        );
        // Where it goes.
        assert_eq!(target(&new_file, r" a\b.rs "), "src/a/b.rs");
        assert_eq!(target(&rename, "lib.rs"), "src/lib.rs");
        assert_eq!(
            target(
                &Naming::New {
                    dir: String::new(),
                    folder: true
                },
                "docs"
            ),
            "docs"
        );
    }

    #[test]
    fn a_rename_selects_the_name_before_its_extension() {
        assert_eq!(stem_bytes("main.rs", false), 4);
        assert_eq!(stem_bytes("驱动.rs", false), "驱动".len());
        assert_eq!(stem_bytes(".env", false), 4);
        assert_eq!(stem_bytes("archive.tar.gz", false), "archive.tar".len());
        assert_eq!(stem_bytes("v1.2", true), 4, "a folder's whole name");
    }
}
