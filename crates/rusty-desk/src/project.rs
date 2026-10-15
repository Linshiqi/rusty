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
    /// The part, as the catalogue the project sees describes it: the
    /// built-in one, the user's, and the project's own `.rusty/chips/`.
    pub chip: Option<rusty_embed::Chip>,
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
        let embedded = detect(&root);
        let chip = embedded
            .as_ref()
            .ok()
            .and_then(|p| p.chip.as_deref())
            .and_then(|id| {
                rusty_embed::catalog::Catalog::load(Some(&root))
                    .chip(id)
                    .cloned()
            });
        Ok(Project {
            root,
            name,
            entries,
            expanded: BTreeSet::new(),
            files,
            embedded,
            chip,
        })
    }

    /// The tree read again after the window changed it: the finder's
    /// files with it, the open folders as they were.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        let mut files = Vec::new();
        collect_files(&entries, &mut files);
        self.entries = entries;
        self.files = files;
    }

    /// The rows shown: every entry whose folders are all open.
    pub fn rows(&self) -> Vec<ShownRow> {
        let mut rows = Vec::new();
        walk(&self.entries, 0, &self.expanded, &mut rows);
        rows
    }
}

/// What the project at `root` is, as the window reads it: detected where
/// it builds — the firmware crate one folder down in the standard layout,
/// whose chip is the project's — and named for the folder that was opened.
/// A playground is marked as one, and the drawing playground, a host crate
/// on purpose, is not told it lacks a chip.
fn detect(root: &std::path::Path) -> Result<rusty_embed::EmbeddedProject, String> {
    let firmware = rusty_embed::project::firmware_root(root);
    let mut project = rusty_embed::project::detect(&firmware).map_err(|error| error.to_string())?;
    if firmware != root {
        if let Ok(sub) = firmware.strip_prefix(root) {
            let sub = sub.to_string_lossy().replace('\\', "/");
            let where_from = format!("in {sub}/");
            project.chip_source = Some(match project.chip_source {
                Some(source) => format!("{source}, {where_from}"),
                None => where_from,
            });
            project.firmware_dir = Some(sub);
        }
        project.root = root.display().to_string();
    }
    project.playground = rusty_embed::config::data_dir()
        .and_then(|data| rusty_embed::playground::chip_of(&data, root))
        .map(str::to_string);
    if project.playground.as_deref() == Some(rusty_embed::PLAYGROUND_DRAW) {
        project
            .problems
            .retain(|problem| problem.kind != "chip-none");
    }
    Ok(project)
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
            chip: None,
        };
        let names = |p: &Project| p.rows().into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(&project), ["src", "Cargo.toml"]);
        let mut open = project;
        open.expanded.insert("src".into());
        assert_eq!(names(&open), ["src", "main.rs", "Cargo.toml"]);
        assert_eq!(open.rows()[1].depth, 1);
    }
}
