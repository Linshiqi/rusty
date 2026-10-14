//! The project as a part: which chip it is for and what the window can run
//! it on, both read off the catalogue — the simulator where the part has an
//! emulation, the board where it has a way to be written, neither invented
//! — and what stands between the project and a build, in the Problems
//! panel with the fix each problem names.

use std::rc::Rc;

use rusty_embed::{BuildSystem, Chip, EmbeddedProject, Problem, Severity};
use rusty_i18n::t;
use slint::{ComponentHandle, SharedString};

use super::App;
use crate::Shell;

/// What the title bar's run control runs on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// No project open: nothing to run.
    Nothing,
    /// A folder no build system claims.
    NotAProject,
    /// A project whose chip nothing names.
    NoPart,
    /// rusty's emulator of the part.
    Simulator,
    /// The board, through one of the part's flashers or a probe.
    Board(String),
    /// A part the catalogue knows no way to run: named, and nothing more.
    Nowhere(String),
}

/// The target a project runs on, by what its part can do. The simulator
/// runs firmware cargo builds, so a PlatformIO or CMake project with a part
/// QEMU models still runs on the board.
pub fn target_of(project: Option<&Result<EmbeddedProject, String>>, chip: Option<&Chip>) -> Target {
    let Some(project) = project else {
        return Target::Nothing;
    };
    let Ok(project) = project else {
        return Target::NotAProject;
    };
    let Some(chip) = chip else {
        return Target::NoPart;
    };
    if chip.emulation.is_some() && project.build.system == BuildSystem::Cargo {
        Target::Simulator
    } else if !chip.flashers.is_empty() || chip.probe_rs_target.is_some() {
        Target::Board(chip.name.clone())
    } else {
        Target::Nowhere(chip.name.clone())
    }
}

/// A project problem's title and detail in the window's language: looked up
/// by its `kind` and refilled from its `args`, the English kept where the
/// catalogue has no entry — a sentence rather than a key.
pub fn problem_text(problem: &Problem) -> (String, String) {
    let one = |suffix: &str, english: &str| {
        rusty_i18n::translate(&format!("problem.{}-{suffix}", problem.kind)).map_or_else(
            || english.to_string(),
            |text| {
                let args: Vec<(&str, String)> = problem
                    .args
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.clone()))
                    .collect();
                rusty_i18n::fill(&text, &args)
            },
        )
    };
    (one("title", &problem.title), one("detail", &problem.detail))
}

/// How a project problem is drawn and counted: as an error, a warning or
/// something to know.
pub fn level_of(severity: Severity) -> i32 {
    match severity {
        Severity::Blocking => 1,
        Severity::Warning => 2,
        Severity::Info => 3,
    }
}

impl App {
    /// The chip in the status bar, and the run target in the title bar.
    pub(super) fn sync_part(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let project = state.project.as_ref();
        let chip = project.and_then(|p| p.chip.as_ref());
        let target = target_of(project.map(|p| &p.embedded), chip);
        let (chip_name, chip_tip) = match (chip, project.and_then(|p| p.embedded.as_ref().ok())) {
            (Some(chip), Some(found)) => {
                let mut tip = format!(
                    "{} · {} · {}",
                    chip.name,
                    chip.vendor_name,
                    found.build.system.label()
                );
                if let Some(source) = &found.chip_source {
                    tip.push_str(" · ");
                    tip.push_str(source);
                }
                (chip.name.clone(), tip)
            }
            _ => (String::new(), String::new()),
        };
        shell.set_chip(chip_name.into());
        shell.set_chip_tip(chip_tip.into());
        // What somebody chose wins over what the part makes of it.
        if let Some((text, kind)) = self.chosen_target() {
            shell.set_target(SharedString::from(text));
            shell.set_target_kind(SharedString::from(kind));
            return;
        }
        shell.set_target_kind(SharedString::from(match target {
            Target::Simulator => "simulator",
            Target::Board(_) => "board",
            _ => "",
        }));
        shell.set_target(SharedString::from(match target {
            Target::Nothing => String::new(),
            Target::NotAProject => t!("desk.target-not-a-project"),
            Target::NoPart => t!("desk.target-no-part"),
            Target::Simulator => t!("desk.simulator"),
            Target::Board(name) => t!("desk.target-board", chip = name),
            Target::Nowhere(name) => name,
        }));
    }

    /// The project read again — a fix ran, and what stood between it and a
    /// build may be gone — keeping the tree as it was opened.
    pub(super) fn reread_project(self: &Rc<Self>) {
        let Some(root) = self.root() else { return };
        crate::work::spawn(
            move || crate::project::Project::read(root),
            |app, read| {
                let Ok(mut fresh) = read else { return };
                {
                    let mut state = app.state.borrow_mut();
                    let Some(project) = state.project.as_mut() else {
                        return;
                    };
                    if project.root != fresh.root {
                        return;
                    }
                    fresh.expanded = std::mem::take(&mut project.expanded);
                    *project = fresh;
                }
                app.sync_part();
                app.sync_problems();
                app.sync_tree();
            },
        );
    }

    /// The project's problem at `index` clicked: what it says, and its fix
    /// to run when it names one.
    pub(super) fn project_problem(self: &Rc<Self>, index: usize) {
        let problem = {
            let state = self.state.borrow();
            state
                .project
                .as_ref()
                .and_then(|p| p.embedded.as_ref().ok())
                .and_then(|p| p.problems.get(index).cloned())
        };
        let Some(problem) = problem else { return };
        let (title, detail) = problem_text(&problem);
        let level = match problem.severity {
            Severity::Blocking => 2,
            Severity::Warning => 1,
            Severity::Info => 0,
        };
        let action = problem.fix_command.map(super::NoteAction::Run);
        self.note_with(level, format!("{title}\n{detail}"), action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real project of the build system's, detected as the window does.
    fn project(system: BuildSystem) -> Result<EmbeddedProject, String> {
        let dir = tempfile::tempdir().unwrap();
        let (file, text) = match system {
            BuildSystem::Cargo => (
                "Cargo.toml",
                "[package]\nname = \"p\"\nversion = \"0.1.0\"\n",
            ),
            BuildSystem::Cmake => (
                "CMakeLists.txt",
                "cmake_minimum_required(VERSION 3.20)\nproject(p C)\n",
            ),
            BuildSystem::PlatformIo => ("platformio.ini", "[env:p]\n"),
        };
        std::fs::write(dir.path().join(file), text).unwrap();
        let found = rusty_embed::project::detect(dir.path()).map_err(|e| e.to_string());
        assert_eq!(found.as_ref().map(|p| p.build.system), Ok(system));
        found
    }

    /// Chosen from the catalogue by what each can do, never by name: the
    /// rule is about capabilities, and so is the test.
    #[test]
    fn the_target_is_what_the_part_can_do() {
        let catalog = rusty_embed::catalog::Catalog::builtin();
        let emulated = catalog
            .chips()
            .iter()
            .find(|c| c.emulation.is_some())
            .expect("a part rusty emulates");
        let flashed_only = catalog
            .chips()
            .iter()
            .find(|c| c.emulation.is_none() && !c.flashers.is_empty())
            .expect("a part only a board runs");
        let cargo = project(BuildSystem::Cargo);
        assert_eq!(target_of(Some(&cargo), Some(emulated)), Target::Simulator);
        assert_eq!(
            target_of(Some(&project(BuildSystem::Cmake)), Some(emulated)),
            Target::Board(emulated.name.clone()),
            "the simulator runs what cargo builds"
        );
        assert_eq!(
            target_of(Some(&cargo), Some(flashed_only)),
            Target::Board(flashed_only.name.clone())
        );
        assert_eq!(target_of(Some(&cargo), None), Target::NoPart);
        assert_eq!(
            target_of(Some(&Err("no".into())), None),
            Target::NotAProject
        );
        assert_eq!(target_of(None, None), Target::Nothing);
    }
}
