//! Building, as the project's own build system builds it, and every line a
//! run prints on its way to the Output dock.
//!
//! Build writes what is on screen first, then runs the build system's own
//! plan — `cargo build --release`, `pio run`, CMake configured once and then
//! built — in the directory the firmware is built in, a step at a time,
//! stopping at the first that fails. The title bar says what it is doing and
//! then how it went, the image's size in that verdict; the compiler's errors
//! are listed in Problems under the build, and every line that names a place
//! goes there when clicked.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
};

use rusty_embed::{
    CommandPlan,
    activity::{Kind, Size},
    buildlog::{self, Located, Severity},
};
use rusty_i18n::t;

use super::App;
use crate::{
    run::{self, Ended},
    work,
};

/// What the last build said was wrong, and where.
#[derive(Default)]
pub(super) struct BuildLog {
    reader: buildlog::Reader,
    /// Errors placed in files, the paths as the project names them (or
    /// absolute, outside it).
    pub(super) errors: Vec<Located>,
}

/// What a build hands on to whoever asked for it.
pub(super) type Built = Box<dyn FnOnce(&Rc<App>, bool) + Send>;

impl App {
    /// One line a run printed: into the dock, a link when it names a place;
    /// to the activity view; and to the build's reading of its errors.
    pub fn heard(self: &Rc<Self>, text: &str) {
        // A protocol line is read into what the board shows, not printed.
        if text.contains("[rusty:") && self.absorb(text) {
            return;
        }
        let place = buildlog::place_on(text).map(|place| {
            let path = self.place_path(&place.path);
            (path, place.line as i32 - 1, place.col as i32 - 1)
        });
        self.runner.say_at(text, run::kind_of(text), place);
        self.testing_heard(text);
        if self.activity_kind() == Some(Kind::Build) {
            let found = self.state.borrow_mut().build_log.reader.read(text);
            if let Some(mut found) = found
                && found.severity == Severity::Error
            {
                found.path = self.place_path(&found.path);
                self.state.borrow_mut().build_log.errors.push(found);
            }
        }
        self.follow_activity(text);
    }

    /// A path a tool printed, as the project names it: relative to the
    /// directory the tool ran in, and then to the project's root; absolute
    /// when it is outside.
    fn place_path(&self, printed: &str) -> String {
        let path = Path::new(printed);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            match self.runner.dir() {
                Some(dir) => dir.join(path),
                None => return printed.replace('\\', "/"),
            }
        };
        let full = full.to_string_lossy().into_owned();
        self.root()
            .and_then(|root| super::navigate::within(&root, &full))
            .unwrap_or(full)
    }

    /// A line of the dock clicked: the place it names, if it names one.
    pub(super) fn output_click(self: &Rc<Self>, index: usize) {
        let Some(line) = self.runner.line(index) else {
            return;
        };
        if line.path.is_empty() {
            return;
        }
        self.open_place(
            line.path.to_string(),
            (line.line.max(0) as usize, line.col.max(0) as usize),
        );
    }

    /// Build what is on screen; `then` hears whether it built.
    pub(super) fn build(self: &Rc<Self>, then: Built) {
        if self.runner.is_running() {
            self.note_transient(1, t!("desk.already-running"));
            return;
        }
        let Some(root) = self.root() else { return };
        self.write_all_then(move |_| {
            work::spawn(
                move || {
                    let firmware = rusty_embed::project::firmware_root(&root);
                    let detected = rusty_embed::project::detect(&firmware)?;
                    let wrote = rusty_embed::buildsys::ensure_toolchain_file(&detected, &firmware)?;
                    let plans = rusty_embed::buildsys::build_plans(&detected, &firmware)?;
                    Ok::<_, rusty_embed::Error>((firmware, detected.chip, wrote, plans))
                },
                move |app, planned| match planned {
                    Ok((firmware, chip, wrote, plans)) => {
                        app.runner.clear();
                        app.state.borrow_mut().build_log = BuildLog::default();
                        app.sync_problems();
                        app.begin(Kind::Build, None);
                        if let Some(file) = wrote {
                            app.runner
                                .say(&t!("desk.wrote-toolchain", file = file), run::kind::PLAIN);
                        }
                        app.build_steps(plans.into(), firmware, chip, then);
                    }
                    Err(error) => {
                        app.note(2, error.to_string());
                        then(app, false);
                    }
                },
            );
        });
    }

    /// The plan's steps in turn, the first failure ending them.
    fn build_steps(
        self: &Rc<Self>,
        mut plans: VecDeque<CommandPlan>,
        firmware: PathBuf,
        chip: Option<String>,
        then: Built,
    ) {
        let Some(plan) = plans.pop_front() else {
            self.built(Ended::Done, &firmware, chip, then);
            return;
        };
        // A build on a volume with no room dies mid-compile with `IO
        // failure on output stream`, which reads as a broken compiler:
        // refused first, with the number.
        if let Some(free) = rusty_core::disk::too_full_for(&plan.program, &plan.args, &firmware) {
            self.note(
                2,
                t!(
                    "desk.disk-full",
                    free = (free / (1024 * 1024)).to_string(),
                    path = firmware.display().to_string()
                ),
            );
            self.built(Ended::Refused, &firmware, chip, then);
            return;
        }
        let dir = firmware.clone();
        let started = self.run(&plan, Some(dir), move |app, ended| {
            if ended == Ended::Done {
                app.build_steps(plans, firmware, chip, then);
            } else {
                app.built(ended, &firmware, chip, then);
            }
        });
        if !started {
            self.abandon();
        }
    }

    /// The build is over: its verdict, its errors in Problems, and — when
    /// it built — the image weighed against the part's memory.
    fn built(self: &Rc<Self>, ended: Ended, firmware: &Path, chip: Option<String>, then: Built) {
        match ended {
            Ended::Stopped => self.abandon(),
            Ended::Done => self.conclude(Some(0)),
            Ended::Failed(code) => self.conclude(code.or(Some(1))),
            Ended::Refused => self.conclude(Some(1)),
        }
        // Read the tree again: cargo makes `target/` before it writes the
        // `CACHEDIR.TAG` that marks it a build directory, so a watcher that
        // spoke in between listed it — and nothing inside it is watched.
        self.reread_project();
        self.sync_problems();
        if ended != Ended::Done {
            then(self, false);
            return;
        }
        let Some(root) = self.root() else {
            then(self, true);
            return;
        };
        let firmware = firmware.to_path_buf();
        work::spawn(
            move || {
                let image = rusty_embed::firmware::newest_in_project(&root)?;
                rusty_embed::memory::analyze_project(
                    Path::new(&image.path),
                    chip.as_deref(),
                    Some(&firmware),
                )
                .ok()
            },
            move |app, report| {
                if let Some(report) = report {
                    let size = Size {
                        flash: report.totals.flash_bytes,
                        ram: report.totals.ram_bytes,
                        ram_capacity: report.totals.ram_capacity,
                    };
                    app.runner
                        .say(&super::activity::size_line(size), run::kind::PLAIN);
                    app.weighed(size);
                }
                app.memory_after_build();
                app.disk_auto_sweep();
                then(app, true);
            },
        );
    }
}
