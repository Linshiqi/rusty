//! The places a run prints, named as the project names them.
//!
//! cargo runs rustc in the workspace's root, and every path it prints for a
//! package of the workspace — a compile error's `-->`, a panic's `at` — is
//! relative to that root, not to the directory the run started in:
//! `core\src\lib.rs` from the root and from inside `core/` alike. Read
//! against the run's directory, a project opened at its member `core/` had
//! every link in the dock one directory too deep. So the root is asked of
//! cargo for each directory a run starts in (`host_debug::workspace_root`,
//! `cargo locate-project --workspace`), off the UI thread, and kept: a
//! directory with no workspace — a CMake or PlatformIO project's — is its
//! own base, as it always was.
//!
//! A line heard while its directory's root is still being asked is said
//! without its link, and given it when cargo answers (`Runner::relink`),
//! as is an error the build listed meanwhile: never a link to the wrong
//! file in the meantime.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use rusty_embed::host_debug;

use super::App;
use crate::work;

/// Told the workspace's root of a directory once cargo has said it.
type Then = Box<dyn FnOnce(&Rc<App>, Option<PathBuf>)>;

enum Base {
    /// Being asked, with who is waiting for the answer.
    Asking(Vec<Then>),
    /// cargo's answer: `None` where it named no workspace.
    Known(Option<PathBuf>),
}

#[derive(Default)]
pub(super) struct Places {
    /// By the directory a run started in.
    bases: HashMap<PathBuf, Base>,
    /// Lines of the dock heard while their directory's root was asked:
    /// the line's number, the directory, and the place as printed.
    waiting: Vec<(u64, PathBuf, String, i32, i32)>,
}

impl Places {
    /// What `dir`'s relative paths are relative to: `None` while cargo is
    /// being asked, the directory itself where it named no workspace.
    fn base_of(&self, dir: &Path) -> Option<PathBuf> {
        match self.bases.get(dir)? {
            Base::Known(base) => Some(base.clone().unwrap_or_else(|| dir.to_path_buf())),
            Base::Asking(_) => None,
        }
    }
}

impl App {
    /// The workspace's root for `dir` — the directory rustc runs in — from
    /// what is kept, or asked of cargo off the UI thread; `then` hears it
    /// either way, `None` where cargo named no workspace.
    pub(super) fn cargo_base(
        self: &Rc<Self>,
        dir: PathBuf,
        then: impl FnOnce(&Rc<App>, Option<PathBuf>) + 'static,
    ) {
        let known = match self.state.borrow().places.bases.get(&dir) {
            Some(Base::Known(base)) => Some(base.clone()),
            _ => None,
        };
        if let Some(base) = known {
            return then(self, base);
        }
        {
            let mut state = self.state.borrow_mut();
            if let Some(Base::Asking(waiting)) = state.places.bases.get_mut(&dir) {
                waiting.push(Box::new(then));
                return;
            }
            state
                .places
                .bases
                .insert(dir.clone(), Base::Asking(vec![Box::new(then)]));
        }
        let asked = dir.clone();
        work::spawn(
            move || host_debug::workspace_root(&asked).ok(),
            move |app, base| app.cargo_base_known(dir, base),
        );
    }

    /// `dir`'s workspace root is `base`: kept, everybody waiting for it
    /// told, and the dock's lines and the build's errors that waited for
    /// it given their places.
    pub(super) fn cargo_base_known(self: &Rc<Self>, dir: PathBuf, base: Option<PathBuf>) {
        let waiting = {
            let mut state = self.state.borrow_mut();
            match state
                .places
                .bases
                .insert(dir.clone(), Base::Known(base.clone()))
            {
                Some(Base::Asking(waiting)) => waiting,
                _ => Vec::new(),
            }
        };
        for then in waiting {
            then(self, base.clone());
        }
        self.relink(&dir);
    }

    /// What was kept is about the manifests as they were: a project
    /// switched, or a `Cargo.toml` that changed, is asked again.
    pub(super) fn forget_cargo_bases(&self) {
        let mut state = self.state.borrow_mut();
        state
            .places
            .bases
            .retain(|_, base| matches!(base, Base::Asking(_)));
    }

    /// A path a tool printed, as the project names it — relative to the
    /// workspace's root of the directory it ran in, and then to the
    /// project's root; absolute when it is outside — or `None` while that
    /// root is still being asked.
    pub(super) fn place_path(&self, printed: &str) -> Option<String> {
        let root = self.root();
        if absolute(printed) {
            return Some(named(printed, Path::new(""), root.as_deref()));
        }
        let Some(dir) = self.runner.dir() else {
            return Some(printed.replace('\\', "/"));
        };
        let base = self.state.borrow().places.base_of(&dir)?;
        Some(named(printed, &base, root.as_deref()))
    }

    /// A line into the dock, a link when it names a place — or, while the
    /// workspace's root of the run's directory is being asked, a link once
    /// cargo has answered.
    pub(super) fn say_linked(&self, text: &str) {
        let kind = crate::run::kind_of(text);
        let Some(place) = rusty_embed::buildlog::place_on(text) else {
            self.runner.say_at(text, kind, None);
            return;
        };
        let (line, col) = (place.line as i32 - 1, place.col as i32 - 1);
        match self.place_path(&place.path) {
            Some(path) => self.runner.say_at(text, kind, Some((path, line, col))),
            None => {
                self.runner.say_at(text, kind, None);
                if let Some(dir) = self.runner.dir() {
                    let said = self.runner.said();
                    self.state
                        .borrow_mut()
                        .places
                        .waiting
                        .push((said, dir, place.path, line, col));
                }
            }
        }
    }

    /// The lines and errors that waited for `dir`'s root, given their
    /// places.
    fn relink(self: &Rc<Self>, dir: &Path) {
        let Some(base) = self.state.borrow().places.base_of(dir) else {
            return;
        };
        let root = self.root();
        let lines: Vec<_> = {
            let mut state = self.state.borrow_mut();
            let (ours, rest) = std::mem::take(&mut state.places.waiting)
                .into_iter()
                .partition(|(_, at, ..)| at == dir);
            state.places.waiting = rest;
            ours
        };
        for (said, _, printed, line, col) in lines {
            let path = named(&printed, &base, root.as_deref());
            self.runner.relink(said, (path, line, col));
        }
        if self.build_relink(dir, &base, root.as_deref()) {
            self.sync_problems();
        }
    }
}

/// `printed`, a path a tool printed relative to `base`, as the project at
/// `root` names it: relative to the root with forward slashes where it is
/// inside, and the whole path where it is not.
pub(super) fn named(printed: &str, base: &Path, root: Option<&Path>) -> String {
    if let Some(root) = root
        && let Some(inside) =
            rusty_edit::testrun::file_in(printed, &base.to_string_lossy(), &root.to_string_lossy())
    {
        return inside;
    }
    if absolute(printed) {
        printed.to_string()
    } else {
        base.join(printed).to_string_lossy().into_owned()
    }
}

/// Absolute wherever it was printed: a drive letter, or a leading slash —
/// the standard library's `/rustc/<hash>/…`, which Windows calls relative
/// and would join onto the base's drive.
fn absolute(printed: &str) -> bool {
    Path::new(printed).is_absolute()
        || printed.starts_with(['/', '\\'])
        || matches!(printed.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case the dock had wrong: a project opened at its member `core/`,
    /// and cargo printing `core\src\lib.rs` relative to the workspace's
    /// root above it. Read against the workspace's root it is the
    /// project's `src/lib.rs`; read against the run's directory it was
    /// `core/src/lib.rs`, a file the project does not have.
    #[test]
    fn a_members_path_is_read_against_the_workspaces_root() {
        let workspace = Path::new("/w");
        let member = Path::new("/w/core");
        assert_eq!(
            named(r"core\src\lib.rs", workspace, Some(member)),
            "src/lib.rs"
        );
        assert_eq!(
            named("core/src/lib.rs", workspace, Some(workspace)),
            "core/src/lib.rs",
            "opened at the root, the same path is the member's file",
        );
    }

    /// Outside the project — the root package's file seen from a member,
    /// the standard library — a place is the whole path, which is what a
    /// click opens.
    #[test]
    fn a_place_outside_the_project_is_its_whole_path() {
        let member = Path::new("/w/core");
        assert_eq!(
            named("src/main.rs", Path::new("/w"), Some(member)),
            Path::new("/w").join("src/main.rs").to_string_lossy()
        );
        assert_eq!(
            named(
                "/rustc/abc/library/core/src/panicking.rs",
                Path::new("/w"),
                Some(member)
            ),
            "/rustc/abc/library/core/src/panicking.rs"
        );
    }

    /// A tool that is not cargo — CMake's compiler, PlatformIO's — prints
    /// against the directory it ran in, which is then the base.
    #[test]
    fn with_no_workspace_the_runs_directory_is_the_base() {
        let project = Path::new("/p");
        assert_eq!(named("src/main.c", project, Some(project)), "src/main.c");
    }
}
