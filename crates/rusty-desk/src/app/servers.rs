//! The language servers on the window's side: started with the project,
//! what they say routed to the editor, and what they found in the status
//! bar and the Problems panel. `lsp.rs` is the servers themselves.

use std::rc::Rc;

use rusty_i18n::t;
use rusty_lsp::{DiagSeverity, FileDiagnostic, HealthLevel, LspEvent, ServerKind};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{ProblemRow, Shell, editor, lsp, work};

/// What the status bar says about the servers.
#[derive(Default)]
pub(super) struct ServerStatus {
    /// Starting, before anything is known.
    starting: bool,
    /// What it calls the servers running — `rust-analyzer + clangd` — and
    /// empty with none.
    label: String,
    /// What the project's server is busy with: `Indexing 12/45`.
    busy: Option<String>,
    /// What is wrong, in the server's words, and how badly.
    trouble: Option<(HealthLevel, String)>,
}

impl App {
    /// The project's servers, started off this thread; whatever ran for the
    /// last project goes, and nothing it said still counts.
    pub(super) fn start_servers(self: &Rc<Self>) {
        self.servers.stop();
        self.problems.borrow_mut().clear();
        let target = {
            let mut state = self.state.borrow_mut();
            state.server = ServerStatus {
                starting: true,
                ..ServerStatus::default()
            };
            state
                .project
                .as_ref()
                .map(|p| (p.root.clone(), p.embedded.clone().ok()))
        };
        self.sync_server();
        self.sync_problems();
        let Some((root, embedded)) = target else {
            return;
        };
        let generation = self.servers.generation();
        work::spawn(
            move || lsp::start(&root, embedded.as_ref()),
            move |app, started| app.servers_started(generation, started),
        );
    }

    fn servers_started(self: &Rc<Self>, generation: u64, started: lsp::Started) {
        // Another project opened while these were starting: they go, off
        // this thread, since a server takes a moment to shut down.
        if generation != self.servers.generation() {
            std::thread::spawn(move || drop(started));
            return;
        }
        let mut clients = Vec::new();
        let mut refused = Vec::new();
        match started.main {
            Ok(pair) => clients.push(pair),
            Err(refusal) => refused.push(refusal),
        }
        match started.companion {
            Some(Ok(pair)) => clients.push(pair),
            Some(Err(refusal)) => refused.push(refusal),
            None => {}
        }
        self.servers.install(clients);
        // Whatever was opened while they started is told now.
        self.editor.announce_all();
        {
            let mut state = self.state.borrow_mut();
            state.server.starting = false;
            state.server.label = self.servers.label();
            if state.server.label.is_empty()
                && let Some(refusal) = refused.first()
            {
                state.server.label = refusal.server.label().to_string();
                state.server.trouble = Some((HealthLevel::Error, refusal.message.clone()));
            }
        }
        for refusal in refused {
            self.note_once(
                1,
                t!(
                    "desk.server-unavailable",
                    server = refusal.server.label(),
                    reason = refusal.message,
                    fix = refusal.install.unwrap_or_default()
                ),
            );
        }
        self.sync_server();
    }

    /// What a server said. `beside` is clangd running beside the project's
    /// own server, of which only what it found is passed on.
    pub fn lsp_event(
        self: &Rc<Self>,
        generation: u64,
        kind: ServerKind,
        beside: bool,
        event: LspEvent,
    ) {
        if generation != self.servers.generation() || (beside && !lsp::passed_from_beside(&event)) {
            return;
        }
        match event {
            LspEvent::Diagnostics { path, items } => {
                {
                    let mut problems = self.problems.borrow_mut();
                    if items.is_empty() {
                        problems.remove(&path);
                    } else {
                        problems.insert(path.clone(), items);
                    }
                }
                self.editor.problems_changed(&path);
                self.problems_soon();
                return;
            }
            LspEvent::Progress { text } => self.state.borrow_mut().server.busy = text,
            LspEvent::Health { level, message } => {
                let message = message.unwrap_or_default();
                if level != HealthLevel::Ok && !message.is_empty() {
                    let server = kind.label();
                    let note = if level == HealthLevel::Error { 2 } else { 1 };
                    self.note_once(
                        note,
                        t!(
                            "desk.server-health",
                            server = server,
                            message = message.clone()
                        ),
                    );
                }
                self.state.borrow_mut().server.trouble =
                    (level != HealthLevel::Ok).then_some((level, message));
            }
            LspEvent::Exited {} => {
                self.state.borrow_mut().server.trouble = Some((
                    HealthLevel::Error,
                    t!("desk.server-exited", server = kind.label()),
                ));
            }
            LspEvent::Unavailable { message, install } => {
                self.note_once(
                    1,
                    t!(
                        "desk.server-unavailable",
                        server = kind.label(),
                        reason = message,
                        fix = install.unwrap_or_default()
                    ),
                );
            }
            // The server's analysis changed: its colours and hints are
            // asked for again.
            LspEvent::Ready {} | LspEvent::Refresh {} => self.editor.analyse_soon(),
            LspEvent::Companion { .. } => {}
        }
        self.sync_server();
    }

    /// The Problems panel and the counts, a moment after the last answer:
    /// a server answers per file and often, and the panel need not be
    /// rebuilt for each.
    fn problems_soon(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.problems_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(120),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.sync_problems();
                }
            },
        );
    }

    /// F8 and Shift+F8: the next problem in the file, and what it says.
    pub(super) fn next_problem(self: &Rc<Self>, forward: bool) {
        if let Some(problem) = self.editor.next_problem(forward) {
            let level = match problem.severity {
                DiagSeverity::Error => 2,
                DiagSeverity::Warning => 1,
                _ => 0,
            };
            self.note_transient(level, problem.message);
        }
    }

    /// A row of the Problems panel clicked: its file, at the problem.
    pub(super) fn problem_click(self: &Rc<Self>, index: usize) {
        let Some(window) = self.window() else { return };
        let rows = window.global::<Shell>().get_problems();
        let Some(row) = slint::Model::row_data(&rows, index) else {
            return;
        };
        // The project's own problems have no file: their row says which.
        if row.path.is_empty() {
            if row.kind != 0 {
                self.project_problem(row.line as usize);
            }
            return;
        }
        if row.kind == 0 {
            self.open_file_at(row.path.to_string(), None);
        } else {
            self.open_place(row.path.to_string(), (row.line as usize, row.col as usize));
        }
    }

    pub(super) fn sync_problems(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let problems = self.problems.borrow();
        let (mut errors, mut warnings) = (0, 0);
        let mut rows = Vec::new();
        // What stands between the project and a build, first: no file, so
        // a row's line is which of the project's problems it is.
        let state = self.state.borrow();
        if let Some(project) = state.project.as_ref()
            && let Ok(found) = &project.embedded
            && !found.problems.is_empty()
        {
            rows.push(ProblemRow {
                kind: 0,
                text: SharedString::from(t!("desk.problems-project")),
                detail: project.name.as_str().into(),
                path: SharedString::new(),
                line: 0,
                col: 0,
                count: found.problems.len() as i32,
            });
            for (index, problem) in found.problems.iter().enumerate() {
                let kind = super::part::level_of(problem.severity);
                match kind {
                    1 => errors += 1,
                    2 => warnings += 1,
                    _ => {}
                }
                let (title, detail) = super::part::problem_text(problem);
                rows.push(ProblemRow {
                    kind,
                    text: title.into(),
                    detail: detail.lines().next().unwrap_or_default().into(),
                    path: SharedString::new(),
                    line: index as i32,
                    col: 0,
                    count: 0,
                });
            }
        }
        // What the last build's compiler said, which a server's check does
        // not always see: C a build script compiled, a project before its
        // compile database exists. A record of that build, so not counted
        // beside what the servers see now.
        let built = &state.build_log.errors;
        if !built.is_empty() {
            rows.push(ProblemRow {
                kind: 0,
                text: SharedString::from(t!("desk.problems-build")),
                detail: SharedString::new(),
                path: SharedString::new(),
                line: -1,
                col: 0,
                count: built.len() as i32,
            });
            for error in built {
                let name = error.path.rsplit(['/', '\\']).next().unwrap_or(&error.path);
                rows.push(ProblemRow {
                    kind: 1,
                    text: error.message.lines().next().unwrap_or_default().into(),
                    detail: format!(
                        "{name} {}",
                        t!(
                            "desk.caret",
                            line = error.line.to_string(),
                            col = error.col.to_string()
                        )
                    )
                    .into(),
                    path: error.path.as_str().into(),
                    line: error.line as i32 - 1,
                    col: error.col as i32 - 1,
                    count: 0,
                });
            }
        }
        drop(state);
        for (path, items) in problems.iter() {
            let mut shown: Vec<&FileDiagnostic> =
                items.iter().filter(|d| editor::is_problem(d)).collect();
            if shown.is_empty() {
                continue;
            }
            shown.sort_by_key(|d| (d.severity, d.start_line, d.start_col));
            let (folder, name) = match path.rfind('/') {
                Some(at) => (&path[..at], &path[at + 1..]),
                None => ("", path.as_str()),
            };
            rows.push(ProblemRow {
                kind: 0,
                text: name.into(),
                detail: folder.into(),
                path: path.as_str().into(),
                line: 0,
                col: 0,
                count: shown.len() as i32,
            });
            for d in shown {
                let kind = match d.severity {
                    DiagSeverity::Error => {
                        errors += 1;
                        1
                    }
                    DiagSeverity::Warning => {
                        warnings += 1;
                        2
                    }
                    _ => 3,
                };
                let said_by = match (&d.source, &d.code) {
                    (Some(source), Some(code)) => format!("{source}({code}) "),
                    (Some(source), None) => format!("{source} "),
                    (None, Some(code)) => format!("{code} "),
                    (None, None) => String::new(),
                };
                rows.push(ProblemRow {
                    kind,
                    text: d.message.lines().next().unwrap_or_default().into(),
                    detail: format!(
                        "{said_by}{}",
                        t!(
                            "desk.caret",
                            line = (d.start_line + 1).to_string(),
                            col = (d.start_col + 1).to_string()
                        )
                    )
                    .into(),
                    path: path.as_str().into(),
                    line: d.start_line as i32,
                    col: d.start_col as i32,
                    count: 0,
                });
            }
        }
        shell.set_errors(errors);
        shell.set_warnings(warnings);
        shell.set_problems(ModelRc::new(VecModel::from(rows)));
    }

    pub(super) fn sync_server(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let status = &state.server;
        let (text, kind, tip) = if status.starting {
            (t!("desk.server-starting"), 1, String::new())
        } else if let Some((level, message)) = &status.trouble {
            let kind = if *level == HealthLevel::Error { 3 } else { 2 };
            (status.label.clone(), kind, message.clone())
        } else if let Some(busy) = &status.busy {
            (format!("{} · {busy}", status.label), 1, busy.clone())
        } else {
            (status.label.clone(), 0, String::new())
        };
        // An icon in the bar: what it is doing, and any trouble, on hover.
        let tip = if tip.is_empty() || text.contains(&tip) {
            text.clone()
        } else {
            format!("{text}\n{tip}")
        };
        shell.set_server(SharedString::from(text));
        shell.set_server_state(kind);
        shell.set_server_tip(SharedString::from(tip));
    }
}
