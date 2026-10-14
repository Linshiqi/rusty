//! The machine against the project: what is installed, what the project
//! cannot build or flash without, and the button that installs it — the
//! Environment page, and the note that says so when a machine cannot build
//! at all. What is missing is `rusty_embed::setup`'s answer, one
//! derivation for every window; this asks, shows and installs.
//!
//! Installs go one at a time, through the runner, into the Output dock, and
//! stop at the first that fails: two `cargo install`s fight over the
//! package cache, and a queue that carried on past a failure would end by
//! reporting a ready machine that is not.

use std::{collections::VecDeque, rc::Rc};

use rusty_embed::{
    CommandPlan, ToolchainReport,
    install::{self, Install},
    setup::{self, Section, Verdict},
};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{App, NoteAction};
use crate::{EnvRow, Shell, run::Ended, work};

/// What the page knows: the last probe, and what is being installed.
#[derive(Default)]
pub(super) struct Environment {
    report: Option<ToolchainReport>,
    probing: bool,
    /// What is still to install, in order; empty with nothing going.
    queue: VecDeque<String>,
    installing: Option<String>,
}

/// A tool's purpose in the window's language: by its name, the English the
/// report carries where the catalogue has no entry.
fn purpose(name: &str, english: &str) -> String {
    rusty_i18n::translate(&format!("tool.{name}")).unwrap_or_else(|| english.to_string())
}

impl App {
    /// Ask the machine again, off this thread: rustup, cargo and every tool
    /// the project's part needs are each a process to start.
    pub(super) fn probe_environment(self: &Rc<Self>) {
        let project = {
            let mut state = self.state.borrow_mut();
            state.environment.probing = true;
            state.project.as_ref().and_then(|p| p.embedded.clone().ok())
        };
        self.sync_environment();
        work::spawn(
            move || rusty_embed::toolchain::report(project.as_ref()),
            |app, report| {
                let blocked = setup::blocked(&report);
                {
                    let mut state = app.state.borrow_mut();
                    state.environment.probing = false;
                    state.environment.report = Some(report);
                }
                app.sync_environment();
                // The first run: a machine that cannot build says so once,
                // with the page that fixes it one click away. A tool that
                // is only optional is not worth interrupting anybody for.
                if blocked {
                    app.note_once_with(
                        2,
                        t!("desk.cannot-build"),
                        Some(NoteAction::Act("environment.open")),
                    );
                }
            },
        );
    }

    /// Install one tool, or every one this project needs and rusty can
    /// fetch: each in turn, the first failure ending the run.
    pub(super) fn install_tools(self: &Rc<Self>, names: Vec<String>) {
        {
            let mut state = self.state.borrow_mut();
            let environment = &mut state.environment;
            for name in names {
                if !environment.queue.contains(&name)
                    && environment.installing.as_ref() != Some(&name)
                {
                    environment.queue.push_back(name);
                }
            }
        }
        if self.state.borrow().environment.installing.is_none() {
            self.install_next();
        }
    }

    pub(super) fn install_missing(self: &Rc<Self>) {
        let names: Vec<String> = {
            let state = self.state.borrow();
            let Some(report) = state.environment.report.as_ref() else {
                return;
            };
            setup::needed(report)
                .into_iter()
                .filter(|name| setup::can_install(report, name))
                .collect()
        };
        self.install_tools(names);
    }

    fn install_next(self: &Rc<Self>) {
        let next = {
            let mut state = self.state.borrow_mut();
            let next = state.environment.queue.pop_front();
            state.environment.installing = next.clone();
            next
        };
        self.sync_environment();
        let Some(name) = next else {
            // Done, or stopped: what the machine is now, the project's
            // problems with it, and a server that may have been missing.
            self.probe_environment();
            self.reread_project();
            if self.servers.label().is_empty() {
                self.start_servers();
            }
            return;
        };
        // A target is the setup plan's own command; everything else is the
        // tool's install, downloaded or run.
        if let Some(triple) = name.strip_prefix("target:") {
            let plan = CommandPlan::new(
                "rustup",
                vec!["target".into(), "add".into(), triple.into()],
                String::new(),
            );
            let dir = self.root();
            self.run(&plan, dir, |app, ended| app.installed(ended));
            return;
        }
        match install::install(&name) {
            Ok(Install::Commands(plans)) => self.run_in_turn(plans.into(), None),
            Ok(Install::Download(download)) => {
                self.runner.say(
                    &format!(
                        "$ download {}",
                        download.urls.first().cloned().unwrap_or_default()
                    ),
                    crate::run::kind::COMMAND,
                );
                self.show_panel("bottom", "output");
                let (urls, archive) = (download.urls.clone(), download.archive.clone());
                let extract = download.extract.clone();
                work::spawn(
                    move || {
                        install::download(&urls, &archive, |line| {
                            work::deliver(line, |app, line| {
                                app.runner.say(&line, crate::run::kind::PLAIN)
                            });
                        })
                        .map(|()| archive)
                        .map_err(|error| error.to_string())
                    },
                    move |app, fetched| match fetched {
                        Ok(archive) => app.run_in_turn(VecDeque::from([extract]), Some(archive)),
                        Err(reason) => {
                            app.runner.say(&reason, crate::run::kind::ERROR);
                            app.installed(Ended::Failed(None));
                        }
                    },
                );
            }
            Err(error) => {
                self.runner.say(&error.to_string(), crate::run::kind::ERROR);
                self.installed(Ended::Refused);
            }
        }
    }

    /// Run `plans` one after another, the first failure ending them; an
    /// archive they unpacked goes once they have.
    fn run_in_turn(
        self: &Rc<Self>,
        mut plans: VecDeque<CommandPlan>,
        archive: Option<std::path::PathBuf>,
    ) {
        let Some(plan) = plans.pop_front() else {
            if let Some(archive) = archive {
                let _ = std::fs::remove_file(archive);
            }
            self.installed(Ended::Done);
            return;
        };
        self.run(&plan, None, move |app, ended| {
            if ended == Ended::Done {
                app.run_in_turn(plans, archive);
            } else {
                app.installed(ended);
            }
        });
    }

    /// One install over: the next, or the queue dropped at a failure.
    fn installed(self: &Rc<Self>, ended: Ended) {
        if ended != Ended::Done {
            let name = self.state.borrow_mut().environment.installing.take();
            self.state.borrow_mut().environment.queue.clear();
            if let Some(name) = name {
                self.note(2, t!("desk.install-failed", tool = name));
            }
        }
        self.install_next();
    }

    pub(super) fn sync_environment(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let environment = &state.environment;
        shell.set_env_probing(environment.probing);
        shell.set_env_installing(SharedString::from(
            environment
                .installing
                .as_ref()
                .map(|name| t!("environment.installing", tool = name.clone()))
                .unwrap_or_default(),
        ));
        let Some(report) = environment.report.as_ref() else {
            shell.set_env_rows(ModelRc::default());
            shell.set_env_verdict(SharedString::new());
            return;
        };
        let has_project = state.project.is_some();
        let steps = setup::plan(report);
        let manual = |tool: &str| {
            steps
                .iter()
                .find(|s| s.tool == tool)
                .and_then(|s| s.manual.clone())
                .unwrap_or_default()
        };
        let (kind, headline, detail, can_install) = match setup::verdict(report) {
            Verdict::NeedsRust => (
                2,
                t!("environment.needs-rust"),
                format!(
                    "{}\n{}",
                    t!("environment.needs-rust-detail"),
                    manual("rustup")
                ),
                false,
            ),
            Verdict::NeedsLinker => (
                2,
                t!("environment.needs-linker"),
                format!(
                    "{}\n{}",
                    t!("environment.needs-linker-detail"),
                    manual("msvc")
                ),
                false,
            ),
            Verdict::Missing(names) => {
                let installable: Vec<&String> = names
                    .iter()
                    .filter(|n| setup::can_install(report, n))
                    .collect();
                let yourself: Vec<&str> = names
                    .iter()
                    .filter(|n| !setup::can_install(report, n))
                    .map(String::as_str)
                    .collect();
                let count = names.len().to_string();
                (
                    1,
                    if has_project {
                        t!("environment.missing", count = count)
                    } else {
                        t!("environment.missing-machine", count = count)
                    },
                    if yourself.is_empty() {
                        String::new()
                    } else {
                        t!("environment.install-yourself", tools = yourself.join(", "))
                    },
                    !installable.is_empty(),
                )
            }
            Verdict::Ready(optional) => (
                0,
                if has_project {
                    t!("environment.ready")
                } else {
                    t!("environment.ready-machine")
                },
                if optional > 0 {
                    t!("environment.optional-missing", count = optional.to_string())
                } else {
                    String::new()
                },
                false,
            ),
        };
        let needed = setup::needed(report);
        let mut rows: Vec<EnvRow> = Vec::new();
        for (section, title) in [
            (Section::Build, "environment.group-build"),
            (Section::Device, "environment.group-device"),
            (Section::Editor, "environment.group-editor"),
        ] {
            let mut group: Vec<EnvRow> = report
                .status
                .tools
                .iter()
                .filter(|tool| setup::section_of(&tool.name) == section)
                .map(|tool| EnvRow {
                    header: false,
                    name: tool.name.as_str().into(),
                    purpose: purpose(&tool.name, &tool.purpose).into(),
                    state: match &tool.version {
                        Some(version) => version.as_str().into(),
                        None if tool.is_installed() => t!("environment.installed").into(),
                        None if needed.contains(&tool.name) => t!("environment.missing-one").into(),
                        None => t!("environment.not-installed").into(),
                    },
                    installed: tool.is_installed(),
                    needed: needed.contains(&tool.name),
                    installable: !tool.is_installed() && setup::can_install(report, &tool.name),
                    command: tool.install_command.as_str().into(),
                })
                .collect();
            // The chip's target with the tools that build: what a part's
            // Rust compiles for, which rustup adds and nothing else does.
            if section == Section::Build
                && let Some(target) = &report.required_target
                && !report.builds_std
            {
                let name = format!("target:{target}");
                group.insert(
                    0,
                    EnvRow {
                        header: false,
                        name: t!("environment.target-named", target = target.clone()).into(),
                        purpose: t!("environment.target").into(),
                        state: if report.required_target_installed {
                            t!("environment.installed").into()
                        } else {
                            t!("environment.missing-one").into()
                        },
                        installed: report.required_target_installed,
                        needed: needed.contains(&name),
                        installable: !report.required_target_installed
                            && setup::can_install(report, &name),
                        command: name.as_str().into(),
                    },
                );
            }
            if group.is_empty() {
                continue;
            }
            rows.push(EnvRow {
                header: true,
                name: rusty_i18n::lookup(title).into(),
                ..EnvRow::default()
            });
            rows.extend(group);
        }
        shell.set_env_kind(kind);
        shell.set_env_verdict(headline.into());
        shell.set_env_detail(detail.trim().into());
        shell.set_env_can_install(can_install && environment.installing.is_none());
        shell.set_env_rows(ModelRc::new(VecModel::from(rows)));
    }

    /// A row's Install: by the row's own name, a target's as the setup plan
    /// spells it.
    pub(super) fn env_install(self: &Rc<Self>, command: &str, name: &str) {
        let tool = if command.starts_with("target:") {
            command.to_string()
        } else {
            name.to_string()
        };
        self.install_tools(vec![tool]);
    }
}
