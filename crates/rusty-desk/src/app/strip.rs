//! The tab strip outlives the window. Which files a project had open, in
//! which group and which in front, is written to `workbench.toml` as the
//! strip changes and put back when the project opens again — with the
//! drafts kept since, read in the same pass (`drafts.rs`). And what a tab
//! says beside its name when another tab shares the name.

use std::{path::Path, rc::Rc};

use rusty_edit::text_file::{self, TextFile};
use rusty_embed::model::ProjectTabs;

use super::{App, Tab};
use crate::editor::{Document, backup::Backup};

/// What a project's opening reads off the UI thread: the drafts kept for
/// it, each with the disk's text under it, the strip it had and the text
/// of every file on it a draft does not already carry. A file gone or no
/// longer text is left out, and so leaves the strip quietly — a remembered
/// tab is not a file anybody asked for.
pub(super) struct Back {
    pub drafts: Vec<(Backup, String)>,
    pub strip: Option<ProjectTabs>,
    pub files: Vec<Document>,
}

impl Back {
    /// Read on the drafts' thread, beside the drafts.
    pub fn read(root: &Path, drafts: Vec<Backup>) -> Back {
        let drafts: Vec<(Backup, String)> = drafts
            .into_iter()
            .map(|draft| {
                // A file gone or no longer text has nothing under the
                // draft: saving it writes the draft whole.
                let disk = match text_file::read_text(root, &draft.path) {
                    Ok(TextFile::Text { text, .. }) => text,
                    _ => String::new(),
                };
                (draft, disk)
            })
            .collect();
        let strip = rusty_embed::config::tabs_for(&root.to_string_lossy());
        let mut files = Vec::new();
        if let Some(strip) = &strip {
            for path in strip.tabs.iter().chain(&strip.second) {
                let drafted = drafts.iter().any(|(draft, _)| &draft.path == path);
                if drafted || files.iter().any(|d: &Document| &d.path == path) {
                    continue;
                }
                if let Ok(TextFile::Text {
                    text,
                    charset,
                    ending,
                }) = text_file::read_text(root, path)
                {
                    files.push(Document::new(path.clone(), text, charset, ending));
                }
            }
        }
        Back {
            drafts,
            strip,
            files,
        }
    }
}

impl App {
    /// The strip as `workbench.toml` keeps it: files only, the left group
    /// first, whichever has the keys. A macro's expansion is no file, and
    /// there would be nothing to put back.
    fn strip_now(&self) -> Option<ProjectTabs> {
        let state = self.state.borrow();
        let project = state.project.as_ref()?;
        let files = |tabs: &[Tab]| -> Vec<String> {
            tabs.iter()
                .filter_map(|tab| match tab {
                    Tab::File(path) if !rusty_edit::is_expansion(path) => Some(path.clone()),
                    _ => None,
                })
                .collect()
        };
        let front = |tabs: &[Tab], at: Option<usize>| match at.and_then(|at| tabs.get(at)) {
            Some(Tab::File(path)) if !rusty_edit::is_expansion(path) => Some(path.clone()),
            _ => None,
        };
        let (left, left_at, right, right_at) = if state.focus == 0 {
            (&state.tabs, state.active, &state.other, state.other_active)
        } else {
            (&state.other, state.other_active, &state.tabs, state.active)
        };
        let split = state.split;
        Some(ProjectTabs {
            root: project.root.to_string_lossy().into_owned(),
            tabs: files(left),
            active: front(left, left_at),
            second: if split { files(right) } else { Vec::new() },
            second_active: if split { front(right, right_at) } else { None },
        })
    }

    /// The strip may have changed: written, off this thread, when it has.
    /// Not before the project's own strip is back, or opening a project
    /// would write its empty strip over the one it is about to read.
    pub(super) fn strip_changed(&self) {
        if !self.strip_back.get() {
            return;
        }
        let Some(now) = self.strip_now() else { return };
        if self.strip_saved.borrow().as_ref() == Some(&now) {
            return;
        }
        *self.strip_saved.borrow_mut() = Some(now.clone());
        // One writer at a time, in the order asked: two writes on the
        // pool could land the older strip last.
        self.keeper
            .then(move || rusty_embed::config::record_tabs(now));
    }

    /// The project is opening: its strip is not written until it is back.
    pub(super) fn strip_opening(&self) {
        self.strip_back.set(false);
        *self.strip_saved.borrow_mut() = None;
    }

    /// Put back what `Back::read` found, if `root` is still the project.
    /// Into a strip nobody has touched since the project opened, the
    /// remembered one, split as it was; beside files opened meanwhile, the
    /// remembered files after them, and what is in front stays in front.
    pub(super) fn came_back(self: &Rc<Self>, root: &Path, back: Back) {
        // Another project opened meanwhile: the drafts stay kept for next
        // time, and its strip is its own.
        if self.root().as_deref() != Some(root) {
            return;
        }
        let Back {
            drafts,
            strip,
            files,
        } = back;
        for document in files {
            if !self.editor.holds(&document.path) {
                self.editor.add(document);
            }
        }
        let mut drafted = Vec::new();
        for (draft, disk) in drafts {
            let path = draft.path.clone();
            if !self.editor.holds(&path) {
                let document = Document::new(path.clone(), disk, draft.charset, draft.ending);
                self.editor.add(document);
            }
            self.editor.restore(&path, draft.text);
            drafted.push(path);
        }
        let strip = strip.unwrap_or_else(|| ProjectTabs {
            root: String::new(),
            tabs: Vec::new(),
            active: None,
            second: Vec::new(),
            second_active: None,
        });
        let held = |paths: &[String]| -> Vec<Tab> {
            paths
                .iter()
                .filter(|path| self.editor.holds(path))
                .map(|path| Tab::File(path.clone()))
                .collect()
        };
        let (mut left, mut right) = (held(&strip.tabs), held(&strip.second));
        for path in &drafted {
            let tab = Tab::File(path.clone());
            if !left.contains(&tab) && !right.contains(&tab) {
                left.push(tab);
            }
        }
        if left.is_empty() {
            std::mem::swap(&mut left, &mut right);
        }
        let place = |tabs: &[Tab], front: &Option<String>| {
            front
                .as_ref()
                .and_then(|path| tabs.iter().position(|t| *t == Tab::File(path.clone())))
        };
        let untouched = {
            let state = self.state.borrow();
            !state.split && !state.tabs.iter().any(|t| matches!(t, Tab::File(_)))
        };
        {
            let mut state = self.state.borrow_mut();
            if untouched && !left.is_empty() {
                // The front tab: the one in front last time, else the first
                // draft, else the first.
                let first_draft = drafted
                    .first()
                    .and_then(|path| left.iter().position(|t| *t == Tab::File(path.clone())));
                state.active = place(&left, &strip.active).or(first_draft).or(Some(0));
                state.tabs = left;
                if !right.is_empty() {
                    state.other_active = place(&right, &strip.second_active).or(Some(0));
                    state.other = right;
                    (state.split, state.focus) = (true, 0);
                }
            } else {
                for tab in left.into_iter().chain(right) {
                    if !state.tabs.contains(&tab) && !state.other.contains(&tab) {
                        state.tabs.push(tab);
                    }
                }
            }
        }
        self.strip_back.set(true);
        *self.strip_saved.borrow_mut() = self.strip_now();
        if untouched {
            self.editor
                .show_shade(self.shade_file().as_deref(), (0.0, 0.0));
            self.front_tab();
        } else {
            self.sync_tabs();
        }
    }
}

/// What each tab of a strip says beside its name: a page nothing, a file
/// what [`tab_hints`] gives it among the strip's other files, and a macro's
/// expansion — named after its macro — the file and line it was called on.
pub(super) fn strip_hints(tabs: &[Tab]) -> Vec<String> {
    let paths: Vec<String> = tabs
        .iter()
        .filter_map(|tab| match tab {
            Tab::File(path) if !rusty_edit::is_expansion(path) => Some(path.clone()),
            Tab::Picture(path) => Some(path.clone()),
            _ => None,
        })
        .collect();
    let mut hints = tab_hints(&paths).into_iter();
    tabs.iter()
        .map(|tab| match tab {
            Tab::File(path) => match rusty_edit::expansion_parts(path) {
                Some((called, _)) => called.rsplit('/').next().unwrap_or(called).to_string(),
                None => hints.next().unwrap_or_default(),
            },
            Tab::Picture(_) => hints.next().unwrap_or_default(),
            Tab::Page(_) => String::new(),
        })
        .collect()
}

/// What to write beside each tab's name so two tabs are never the same word.
///
/// A workspace has three `Cargo.toml`s, two `main.rs`es and two `lib.rs`es,
/// and a strip showing each as its bare file name was eleven tabs with six
/// labels between them, reported as "I cannot find the file the tab is
/// showing". VS Code's rule: the name alone while it is unique, otherwise as
/// much of the path above it as it takes to tell them apart, no more.
///
/// Parallel to `paths`, empty where the name stands on its own. The suffix
/// grows one directory at a time and stops at the first depth that separates
/// the whole group: `core/Cargo.toml` against `firmware/Cargo.toml` is `core`
/// and `firmware`, and two `lib.rs`es both under a `src` are `core/src` and
/// `firmware/src`, because one segment does not part them.
///
/// A file at the project root keeps a bare name even when it shares one:
/// having nothing above it is what tells it apart, and a word invented for
/// it would be a label the path does not contain.
pub(super) fn tab_hints(paths: &[String]) -> Vec<String> {
    let name_of = |path: &str| path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
    // The directories above the file, nearest first: `core/src/math/q.rs`
    // gives `["math", "src", "core"]`.
    let parents = |path: &str| -> Vec<String> {
        let mut parts: Vec<String> = path.split(['/', '\\']).map(str::to_string).collect();
        parts.pop();
        parts.reverse();
        parts
    };

    let mut hints = vec![String::new(); paths.len()];
    for (index, path) in paths.iter().enumerate() {
        let name = name_of(path);
        let rivals: Vec<usize> = paths
            .iter()
            .enumerate()
            .filter(|(other, p)| *other != index && name_of(p) == name)
            .map(|(other, _)| other)
            .collect();
        if rivals.is_empty() {
            continue;
        }
        let mine = parents(path);
        let deepest = rivals
            .iter()
            .map(|&other| parents(&paths[other]).len())
            .chain(std::iter::once(mine.len()))
            .max()
            .unwrap_or(0);
        for depth in 1..=deepest {
            let suffix = |parts: &[String]| parts.iter().take(depth).cloned().collect::<Vec<_>>();
            let ours = suffix(&mine);
            if rivals
                .iter()
                .all(|&other| suffix(&parents(&paths[other])) != ours)
            {
                // Written the way the path reads, outermost first.
                let mut shown = ours;
                shown.reverse();
                hints[index] = shown.join("/");
                break;
            }
        }
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(paths: &[&str]) -> Vec<String> {
        tab_hints(&paths.iter().map(|p| (*p).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_unique_name_carries_no_suffix() {
        assert_eq!(hints(&["core/src/lib.rs", "firmware/build.rs"]), ["", ""]);
    }

    /// The workspace that produced the report: three `Cargo.toml`s. One
    /// directory apiece parts them, and the one at the root stays bare.
    #[test]
    fn three_manifests_are_told_apart_by_one_directory_each() {
        assert_eq!(
            hints(&["Cargo.toml", "core/Cargo.toml", "firmware/Cargo.toml"]),
            ["", "core", "firmware"]
        );
    }

    #[test]
    fn the_suffix_grows_until_it_separates_them() {
        assert_eq!(
            hints(&["core/src/lib.rs", "firmware/src/lib.rs"]),
            ["core/src", "firmware/src"]
        );
    }

    #[test]
    fn the_suffix_stops_at_the_first_depth_that_works() {
        assert_eq!(
            hints(&["firmware/src/main.rs", "firmware/src/bin/main.rs"]),
            ["src", "bin"]
        );
    }

    #[test]
    fn each_tab_grows_only_as_far_as_its_own_group_needs() {
        assert_eq!(
            hints(&[
                "Cargo.toml",
                "core/Cargo.toml",
                "core/src/lib.rs",
                "firmware/src/lib.rs",
            ]),
            ["", "core", "core/src", "firmware/src"]
        );
    }

    #[test]
    fn a_backslash_is_a_separator_too() {
        assert_eq!(
            hints(&["core\\Cargo.toml", "firmware\\Cargo.toml"]),
            ["core", "firmware"]
        );
    }

    /// Pages say nothing, and do not count as a file's rival.
    #[test]
    fn a_page_between_files_keeps_their_hints_in_place() {
        let tabs = [
            Tab::File("core/Cargo.toml".into()),
            Tab::Page("settings"),
            Tab::File("firmware/Cargo.toml".into()),
        ];
        assert_eq!(strip_hints(&tabs), ["core", "", "firmware"]);
    }

    /// An expansion's tab reads as its macro, and says where it was called:
    /// two of one macro are told apart by their calls, and neither is a
    /// file's rival.
    #[test]
    fn an_expansion_says_where_its_macro_was_called() {
        let tabs = [
            Tab::File("src/main.rs".into()),
            Tab::File(rusty_edit::expansion_path("src/main.rs", 7, "println!")),
            Tab::File(rusty_edit::expansion_path("src/lib.rs", 40, "println!")),
            Tab::File("core/src/main.rs".into()),
        ];
        assert_eq!(tabs[1].title(), "println!");
        assert_eq!(
            strip_hints(&tabs),
            ["src", "main.rs:8", "lib.rs:41", "core/src"]
        );
    }
}
