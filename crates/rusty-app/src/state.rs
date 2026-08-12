use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rusty_core::Workspace;
use rusty_embed::catalog::Catalog;
use tokio::sync::Mutex;

/// What is currently open.
///
/// The Cargo workspace is behind an `Arc` inside the lock so callers can clone
/// the handle and drop the guard immediately. That matters for the assistant: a
/// conversation can run for half a minute, and holding the lock for its
/// duration would freeze every other panel in the window.
#[derive(Default)]
pub struct AppState {
    inner: Mutex<Open>,
    /// The flash or monitor session in flight, if any.
    ///
    /// Only the stopper is kept, not the session: the reader loop blocks on
    /// `recv`, so whoever ends a monitor cannot be the thread sitting inside it.
    session: Mutex<Option<rusty_embed::process::Stopper>>,
    /// Chips and boards, after layering in the user's and the project's files.
    ///
    /// Held here rather than rebuilt per call so every surface — the port list,
    /// the wizard, the assistant's tools — sees exactly the same catalogue. A
    /// board that appears in one panel and not another is a bug report nobody
    /// can reproduce.
    catalog: Mutex<Option<Arc<Catalog>>>,
    /// The open shell, if any.
    ///
    /// One at a time, because the panel shows one. Opening a second replaces
    /// the first — a shell nobody can see is a shell nobody can stop.
    terminal: Mutex<Option<Arc<rusty_term::Terminal>>>,
}

#[derive(Default, Clone)]
struct Open {
    workspace: Option<Arc<Workspace>>,
    /// Project root. Tracked separately from the workspace because embedded
    /// detection reads files directly and must work even when `cargo metadata`
    /// fails — which, for a misconfigured embedded project, is exactly when the
    /// user needs the diagnosis most.
    root: Option<PathBuf>,
    /// The firmware ELF from the last successful build.
    firmware: Option<PathBuf>,
}

impl AppState {
    pub async fn open(&self, root: PathBuf, workspace: Option<Workspace>) {
        // Reload before taking the other lock: a project's `.rusty/` files are
        // part of what "open" means, and a panel that rendered in between would
        // otherwise see the new root with the old board list.
        let catalog = Arc::new(Catalog::load(Some(&root)));
        *self.catalog.lock().await = Some(catalog);

        let mut guard = self.inner.lock().await;
        guard.root = Some(root);
        guard.workspace = workspace.map(Arc::new);
        // A different project's binary is worse than none.
        guard.firmware = None;
    }

    /// The layered catalogue, falling back to the built-ins before anything is
    /// open — the wizard and the chip list are useful with no project at all.
    pub async fn catalog(&self) -> Arc<Catalog> {
        let mut guard = self.catalog.lock().await;
        guard
            .get_or_insert_with(|| Arc::new(Catalog::load(None)))
            .clone()
    }

    pub async fn workspace(&self) -> Option<Arc<Workspace>> {
        self.inner.lock().await.workspace.clone()
    }

    pub async fn root(&self) -> Option<PathBuf> {
        self.inner.lock().await.root.clone()
    }

    pub async fn terminal(&self) -> Option<Arc<rusty_term::Terminal>> {
        self.terminal.lock().await.clone()
    }

    /// Register the open shell, killing whatever it replaces.
    pub async fn set_terminal(&self, terminal: Option<Arc<rusty_term::Terminal>>) {
        let previous = std::mem::replace(&mut *self.terminal.lock().await, terminal);
        if let Some(previous) = previous {
            previous.kill();
        }
    }

    pub async fn set_firmware(&self, path: Option<PathBuf>) {
        self.inner.lock().await.firmware = path;
    }

    /// Register a running session, ending any previous one.
    ///
    /// Two monitors on the same serial port cannot both work — the second gets
    /// an access-denied that reads like a driver problem — so starting one
    /// always stops the last.
    pub async fn start_session(&self, stopper: rusty_embed::process::Stopper) {
        let previous = self.session.lock().await.replace(stopper);
        if let Some(previous) = previous {
            previous.stop();
        }
    }

    pub async fn stop_session(&self) {
        if let Some(stopper) = self.session.lock().await.take() {
            stopper.stop();
        }
    }

    /// Everything a tool call might need, taken in one lock acquisition.
    ///
    /// The lock is released before the filesystem is touched: discovery walks a
    /// target directory, and holding the lock across that would stall every
    /// panel in the window for the duration.
    pub async fn snapshot(&self) -> Snapshot {
        let open = self.inner.lock().await.clone();

        // Fall back to whatever the project has built. Without this the
        // assistant's `memory_report` can only run *after* a human has visited
        // the memory panel — so the first time anyone asks "why is my binary so
        // big", the tool that answers it reports missing context instead.
        let firmware = open.firmware.or_else(|| {
            let root = open.root.as_deref()?;
            let configured = rusty_embed::project::detect(root)
                .ok()
                .and_then(|p| p.configured_target);
            rusty_embed::firmware::newest(root, configured.as_deref())
                .map(|f| PathBuf::from(f.path))
        });

        Snapshot {
            workspace: open.workspace,
            root: open.root,
            firmware,
        }
    }
}

/// A consistent view of what is open, detached from the lock.
pub struct Snapshot {
    pub workspace: Option<Arc<Workspace>>,
    pub root: Option<PathBuf>,
    pub firmware: Option<PathBuf>,
}

impl Snapshot {
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
}
