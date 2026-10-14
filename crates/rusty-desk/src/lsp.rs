//! The language servers: rust-analyzer for a Cargo project's Rust, clangd
//! for a PlatformIO or CMake project's C and C++, and clangd beside
//! rust-analyzer for the C a Cargo project's build script compiles — the
//! three languages read alike, each by the server that knows it
//! (`language.rs` says which reads what).
//!
//! The client blocks on a pipe, so nothing here waits on the UI thread.
//! Three rules carried over from the old window, each the answer to a bug:
//!
//! - **What a document says to a server goes in the order it was said.**
//!   Open, change, close and save are written by one thread from one queue
//!   (`order`); a change is a delta against the text before it, and two
//!   swapped on the wire leave the server holding a text the editor never
//!   had.
//! - **A question carries its text** ([`Draft`], numbered here): a question
//!   asked on a thread of its own can overtake the change before it, so the
//!   draft is given to the server first, in the same thread, and the client
//!   skips a draft older than the one it holds.
//! - **An event belongs to the servers that sent it.** Every start counts a
//!   generation, and an event from a server since replaced — a project
//!   switch, a restart — is dropped on arrival.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::Path,
    rc::Rc,
    sync::{Arc, mpsc},
};

use rusty_embed::{BuildSystem, EmbeddedProject, ToolchainRequirement};
use rusty_lsp::{Draft, Events, FileDiagnostic, LspClient, LspEvent, ServerKind};

use crate::{app::App, language, work};

/// What every server has said about every file: the Problems panel, the
/// status bar's counts and the squiggles all read it.
pub type Problems = Rc<RefCell<BTreeMap<String, Vec<FileDiagnostic>>>>;

type Job = Box<dyn FnOnce() + Send>;

pub struct Servers {
    /// The project's server first, the one beside it second.
    running: RefCell<Vec<Arc<LspClient>>>,
    generation: Cell<u64>,
    order: mpsc::Sender<Job>,
    seq: Cell<u64>,
}

/// A server that could not be started, and what to do about it.
pub struct Refusal {
    pub server: ServerKind,
    pub message: String,
    pub install: Option<String>,
}

/// What starting a project's servers came to.
pub struct Started {
    pub main: Result<(LspClient, Events), Refusal>,
    /// clangd beside rust-analyzer, when the project compiles C.
    pub companion: Option<Result<(LspClient, Events), Refusal>>,
}

/// Where clangd comes from: LLVM's own releases, or the system's package.
const CLANGD_INSTALL: &str =
    "install clangd (https://clangd.llvm.org/installation) and put it on PATH";

/// Start the servers the project at `root` needs. Blocking — a process
/// started, a handshake — so it runs on a worker.
pub fn start(root: &Path, project: Option<&EmbeddedProject>) -> Started {
    let catalog = rusty_embed::catalog::Catalog::load(Some(root));
    let globs = rusty_embed::buildsys::query_driver_globs(&catalog);
    let cargo = project.map_or(root.join("Cargo.toml").is_file(), |p| {
        p.build.system == BuildSystem::Cargo
    });
    let clangd = |database: Option<&Path>| {
        LspClient::spawn_clangd(root, database, &globs, None).map_err(|error| Refusal {
            server: ServerKind::Clangd,
            message: error.to_string(),
            install: Some(CLANGD_INSTALL.to_string()),
        })
    };
    if !cargo {
        // A PlatformIO or CMake project's C is read through the compile
        // database its build writes, with the part's cross compiler asked
        // for its system headers.
        let database = project.and_then(|p| rusty_embed::buildsys::compile_commands_dir(root, p));
        return Started {
            main: clangd(database.as_deref()),
            companion: None,
        };
    }
    let chip = project
        .and_then(|p| p.chip.as_deref())
        .and_then(rusty_embed::chip::by_id);
    // What the firmware builds for, so cfg resolves as for the chip rather
    // than the host — except a target the project carries as a description
    // (a CH32's `.json`), which rust-analyzer's cargo reads from the config
    // for itself, and whose stem no rustc knows as a triple.
    let target = project
        .filter(|_| {
            chip.as_ref()
                .is_none_or(|chip| chip.toolchain != ToolchainRequirement::NightlyBuildStd)
        })
        .and_then(|p| {
            p.configured_target
                .clone()
                .or_else(|| chip.as_ref().map(|chip| chip.bare_metal_target.clone()))
        });
    // A rust-analyzer the user named in `workbench.toml`, when there is one.
    let named = rusty_embed::config::workbench().rust_analyzer;
    let main = LspClient::spawn(root, target.as_deref(), named.as_deref().map(Path::new)).map_err(
        |error| Refusal {
            server: ServerKind::RustAnalyzer,
            message: error.to_string(),
            install: rusty_embed::toolchain::install_command("rust-analyzer"),
        },
    );
    // The C a `cc` build script compiles: a compile database rusty writes
    // under the data directory, naming the part's compiler — `None` when
    // the project has no C, and then no clangd at all.
    let compiler = chip
        .as_ref()
        .and_then(|chip| chip.c_compiler.as_ref())
        .map(|compiler| compiler.binary.clone());
    let companion = rusty_embed::buildsys::write_cargo_compile_commands(root, compiler.as_deref())
        .map(|database| clangd(Some(&database)));
    Started { main, companion }
}

impl Servers {
    pub fn new() -> Rc<Servers> {
        let (order, jobs) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("lsp-order".into())
            .spawn(move || {
                for job in jobs {
                    job();
                }
            })
            .expect("the language servers' thread starts");
        Rc::new(Servers {
            running: RefCell::default(),
            generation: Cell::new(0),
            order,
            seq: Cell::new(0),
        })
    }

    /// The servers that were started take over from whatever ran before,
    /// whose shutdown happens off this thread — a server takes half a
    /// second to go, and a project switch must not wait on it. Their events
    /// are brought to the app under the generation answered.
    pub fn install(&self, clients: Vec<(LspClient, Events)>) -> u64 {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let mut running = Vec::new();
        for (index, (client, events)) in clients.into_iter().enumerate() {
            let kind = client.kind();
            let beside = index > 0;
            std::thread::Builder::new()
                .name(format!("lsp-events-{}", kind.label()))
                .spawn(move || {
                    while let Some(event) = events.recv() {
                        work::deliver(event, move |app, event| {
                            app.lsp_event(generation, kind, beside, event)
                        });
                    }
                })
                .expect("a language server's events thread starts");
            running.push(Arc::new(client));
        }
        let gone = self.running.replace(running);
        if !gone.is_empty() {
            std::thread::spawn(move || drop(gone));
        }
        generation
    }

    /// No project: no servers, and nothing they said still counts.
    pub fn stop(&self) {
        self.install(Vec::new());
    }

    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// The server told about `path`: the one the language table names for
    /// it, among those running — never the other, which would read a C file
    /// as broken Rust.
    fn for_path(&self, path: &str) -> Option<Arc<LspClient>> {
        let server = language::of(path)?.server?;
        let running = self.running.borrow();
        running
            .iter()
            .find(|client| client.kind() == server)
            .cloned()
    }

    fn queue(&self, path: &str, job: impl FnOnce(&LspClient) + Send + 'static) {
        if let Some(client) = self.for_path(path) {
            let _ = self.order.send(Box::new(move || job(&client)));
        }
    }

    /// The editor's text of a file, numbered for the client to order it by.
    pub fn draft(&self, text: &str) -> Draft {
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        Draft {
            text: text.to_string(),
            seq,
        }
    }

    pub fn opened(&self, path: &str, text: &str) {
        let (path, text) = (path.to_string(), text.to_string());
        self.queue(&path.clone(), move |client| {
            let _ = client.did_open(&path, &text);
        });
    }

    pub fn changed(&self, path: &str, text: &str) {
        let draft = self.draft(text);
        let path = path.to_string();
        self.queue(&path.clone(), move |client| {
            let _ = client.sync_draft(&path, &draft);
        });
    }

    pub fn saved(&self, path: &str) {
        let path = path.to_string();
        self.queue(&path.clone(), move |client| {
            let _ = client.did_save(&path);
        });
    }

    /// The file is no longer open: the server reads it off the disk again.
    pub fn closed(&self, path: &str) {
        let path = path.to_string();
        self.queue(&path.clone(), move |client| {
            let _ = client.did_close(&path);
        });
    }

    /// Ask the server for `path` a question on a thread of its own, the
    /// draft it is about given to it first, and hand the answer to `then`
    /// on the UI thread — nothing when no server reads the file, or when
    /// the server could not answer: the warm-up and an edit overtaking the
    /// question both answer with an error, and what is on screen stays.
    pub fn ask<T: Send + 'static>(
        &self,
        path: &str,
        draft: Option<Draft>,
        question: impl FnOnce(&LspClient, &str) -> rusty_lsp::Result<T> + Send + 'static,
        then: impl FnOnce(&Rc<App>, T) + Send + 'static,
    ) {
        let Some(client) = self.for_path(path) else {
            return;
        };
        let path = path.to_string();
        work::spawn(
            move || {
                if let Some(draft) = &draft {
                    client.sync_draft(&path, draft).ok()?;
                }
                question(&client, &path).ok()
            },
            move |app, answer| {
                if let Some(answer) = answer {
                    then(app, answer);
                }
            },
        );
    }

    /// [`Self::ask`] for a question whose failure is news: a rename or a
    /// fix that writes files, where silence would read as done. The
    /// answer, or what went wrong in the client's words.
    pub fn ask_result<T: Send + 'static>(
        &self,
        path: &str,
        question: impl FnOnce(&LspClient, &str) -> rusty_lsp::Result<T> + Send + 'static,
        then: impl FnOnce(&Rc<App>, Result<T, String>) + Send + 'static,
    ) {
        let Some(client) = self.for_path(path) else {
            return;
        };
        let path = path.to_string();
        work::spawn(
            move || question(&client, &path).map_err(|error| error.to_string()),
            then,
        );
    }

    /// What the status bar calls the servers running: `rust-analyzer`, or
    /// `rust-analyzer + clangd` with the one beside it.
    pub fn label(&self) -> String {
        self.running
            .borrow()
            .iter()
            .map(|client| client.kind().label())
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

/// Whether an event from a server beside the project's is passed on: what
/// it found in its files, and nothing about itself — the status bar is the
/// project's server's.
pub fn passed_from_beside(event: &LspEvent) -> bool {
    matches!(event, LspEvent::Diagnostics { .. } | LspEvent::Refresh {})
}
