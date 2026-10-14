//! The open project: its folder, its files as a tree, and the rows of that
//! tree the left dock shows.

use std::{collections::BTreeSet, path::PathBuf};

use rusty_edit::Entry;

pub struct Project {
    pub root: PathBuf,
    pub name: String,
    pub entries: Vec<Entry>,
    /// Folders open in the tree, by path.
    pub expanded: BTreeSet<String>,
    /// Every file, for the finder, in tree order.
    pub files: Vec<String>,
    /// What detection made of it — the build system, the part, what is
    /// missing — or why it could not say.
    pub embedded: Result<rusty_embed::EmbeddedProject, String>,
}

/// A row of the tree as shown: how deep, the entry, and whether it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownRow {
    pub depth: usize,
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub open: bool,
}

impl Project {
    /// Read a project's folder. Slow on a big tree, so it is called off the
    /// UI thread.
    pub fn read(root: PathBuf) -> Result<Project, String> {
        let entries = rusty_edit::read_tree(&root).map_err(|error| error.to_string())?;
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let mut files = Vec::new();
        collect_files(&entries, &mut files);
        let embedded = rusty_embed::project::detect(&root).map_err(|error| error.to_string());
        Ok(Project {
            root,
            name,
            entries,
            expanded: BTreeSet::new(),
            files,
            embedded,
        })
    }

    /// The rows shown: every entry whose folders are all open.
    pub fn rows(&self) -> Vec<ShownRow> {
        let mut rows = Vec::new();
        walk(&self.entries, 0, &self.expanded, &mut rows);
        rows
    }
}

fn walk(entries: &[Entry], depth: usize, expanded: &BTreeSet<String>, out: &mut Vec<ShownRow>) {
    for entry in entries {
        let open = entry.is_dir && expanded.contains(&entry.path);
        out.push(ShownRow {
            depth,
            name: entry.name.clone(),
            path: entry.path.clone(),
            dir: entry.is_dir,
            open,
        });
        if open {
            walk(&entry.children, depth + 1, expanded, out);
        }
    }
}

fn collect_files(entries: &[Entry], out: &mut Vec<String>) {
    for entry in entries {
        if entry.is_dir {
            collect_files(&entry.children, out);
        } else {
            out.push(entry.path.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, dir: bool, children: Vec<Entry>) -> Entry {
        Entry {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            is_dir: dir,
            children,
        }
    }

    #[test]
    fn a_closed_folder_hides_what_is_in_it() {
        let project = Project {
            root: PathBuf::from("."),
            name: "p".into(),
            entries: vec![
                entry("src", true, vec![entry("src/main.rs", false, vec![])]),
                entry("Cargo.toml", false, vec![]),
            ],
            expanded: BTreeSet::new(),
            files: vec![],
            embedded: Err("not read".into()),
        };
        let names = |p: &Project| p.rows().into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(&project), ["src", "Cargo.toml"]);
        let mut open = project;
        open.expanded.insert("src".into());
        assert_eq!(names(&open), ["src", "main.rs", "Cargo.toml"]);
        assert_eq!(open.rows()[1].depth, 1);
    }
}
