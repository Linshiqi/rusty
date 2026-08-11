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
    session: Mutex<Option<rusty_embed::flash::Stopper>>,
    /// Chips and boards, after layering in the user's and the project's files.
    ///
    /// Held here rather than rebuilt per call so every surface — the port list,
    /// the wizard, the assistant's tools — sees exactly the same catalogue. A
    /// board that appears in one panel and not another is a bug report nobody
    /// can reproduce.
    catalog: Mutex<Option<Arc<Catalog>>>,
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

    pub async fn set_firmware(&self, path: Option<PathBuf>) {
        self.inner.lock().await.firmware = path;
    }

    /// Register a running session, ending any previous one.
    ///
    /// Two monitors on the same serial port cannot both work — the second gets
    /// an access-denied that reads like a driver problem — so starting one
    /// always stops the last.
    pub async fn start_session(&self, stopper: rusty_embed::flash::Stopper) {
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
    pub async fn snapshot(&self) -> Snapshot {
        let guard = self.inner.lock().await;
        Snapshot {
            workspace: guard.workspace.clone(),
            root: guard.root.clone(),
            firmware: guard.firmware.clone(),
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
