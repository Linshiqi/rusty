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
    /// The debug session, if one is live.
    debugger: Mutex<Option<Arc<rusty_dbg::Debugger>>>,
    /// The flash or monitor session in flight, if any.
    ///
    /// Only the stopper is kept, not the session: the reader loop blocks on
    /// `recv`, so whoever ends a monitor cannot be the thread sitting inside it.
    session: Mutex<Option<rusty_embed::process::Stopper>>,
    /// The running session's stdin, when it has one worth writing — the
    /// simulator's board input path. Cleared with the session.
    session_input: Mutex<Option<rusty_embed::process::Input>>,
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
    /// Syntax grammars, loaded once.
    ///
    /// `SyntaxSet::load_defaults_newlines` parses a bundled binary dump, which
    /// is slow enough to notice if it happens per file opened.
    files: std::sync::OnceLock<Arc<rusty_edit::Files>>,
    /// The language server for the open project, if one is running.
    ///
    /// Killed and replaced on project switch: rust-analyzer holds the old
    /// project's target directory open, and a server answering questions about
    /// a workspace nobody is looking at is pure cost.
    lsp: Mutex<Option<Arc<rusty_lsp::LspClient>>>,
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
        drop(guard);
        // The old project's server has nothing true left to say.
        self.set_lsp(None).await;
    }

    /// The layered catalogue, falling back to the built-ins before anything is
    /// open — the wizard and the chip list are useful with no project at all.
    pub async fn catalog(&self) -> Arc<Catalog> {
        let mut guard = self.catalog.lock().await;
        guard
            .get_or_insert_with(|| Arc::new(Catalog::load(None)))
            .clone()
    }

    /// Forget the layered catalogue so the next reader rebuilds it — the
    /// user's board files just moved house.
    pub async fn drop_catalog(&self) {
        *self.catalog.lock().await = None;
    }

    pub async fn workspace(&self) -> Option<Arc<Workspace>> {
        self.inner.lock().await.workspace.clone()
    }

    pub async fn root(&self) -> Option<PathBuf> {
        self.inner.lock().await.root.clone()
    }

    pub async fn debugger(&self) -> Option<Arc<rusty_dbg::Debugger>> {
        self.debugger.lock().await.clone()
    }

    /// Register the live debug session, ending whatever it replaces — one
    /// panel, one session, and a stranded gdb holds the ELF open.
    pub async fn set_debugger(&self, debugger: Option<Arc<rusty_dbg::Debugger>>) {
        let previous = std::mem::replace(&mut *self.debugger.lock().await, debugger);
        if let Some(previous) = previous {
            previous.stop();
        }
    }

    /// Release the slot when a session ends, by identity — the same rule the
    /// terminal learned: an outgoing session must not evict its successor.
    pub async fn release_debugger(&self, ours: &Arc<rusty_dbg::Debugger>) {
        let mut slot = self.debugger.lock().await;
        if slot.as_ref().is_some_and(|held| Arc::ptr_eq(held, ours)) {
            *slot = None;
        }
    }

    pub async fn lsp(&self) -> Option<Arc<rusty_lsp::LspClient>> {
        self.lsp.lock().await.clone()
    }

    /// Register the project's language server, dropping — and thereby killing —
    /// whatever it replaces.
    pub async fn set_lsp(&self, client: Option<Arc<rusty_lsp::LspClient>>) {
        *self.lsp.lock().await = client;
    }

    pub fn files(&self) -> Arc<rusty_edit::Files> {
        Arc::clone(
            self.files
                .get_or_init(|| Arc::new(rusty_edit::Files::new())),
        )
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

    /// Release the slot when a session ends — but only if it still holds
    /// *that* session.
    ///
    /// A finished session used to clear the slot unconditionally, and
    /// clearing kills whatever it replaces: switching shells let the old
    /// session's cleanup kill the new session it had just been replaced by,
    /// which is why the terminal came back blank.
    pub async fn release_terminal(&self, ours: &Arc<rusty_term::Terminal>) {
        let mut slot = self.terminal.lock().await;
        if slot.as_ref().is_some_and(|held| Arc::ptr_eq(held, ours)) {
            *slot = None;
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
    pub async fn set_session_input(&self, input: Option<rusty_embed::process::Input>) {
        *self.session_input.lock().await = input;
    }

    pub async fn session_input(&self) -> Option<rusty_embed::process::Input> {
        self.session_input.lock().await.clone()
    }

    pub async fn start_session(&self, stopper: rusty_embed::process::Stopper) {
        let previous = self.session.lock().await.replace(stopper);
        if let Some(previous) = previous {
            previous.stop();
        }
    }

    pub async fn stop_session(&self) {
        *self.session_input.lock().await = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two real shells, the switch-shells sequence: the outgoing session's
    /// cleanup must not evict — or kill — the session that replaced it.
    ///
    /// Real pty sessions rather than mocks, because the bug was in the
    /// identity of the handle, which a mock would have papered over. It
    /// cost two rounds of "the terminal is blank after switching".
    #[tokio::test]
    async fn a_finished_session_does_not_evict_its_successor() {
        let spawn = || {
            let (terminal, _updates) =
                rusty_term::Terminal::spawn(None, 80, 24, None).expect("a shell");
            Arc::new(terminal)
        };
        let state = AppState::default();

        let first = spawn();
        state.set_terminal(Some(Arc::clone(&first))).await;

        // The switch: a new session takes the slot…
        let second = spawn();
        state.set_terminal(Some(Arc::clone(&second))).await;
        // …and only then does the old one finish and clean up.
        state.release_terminal(&first).await;

        let held = state.terminal().await.expect("the new session is still open");
        assert!(
            Arc::ptr_eq(&held, &second),
            "the outgoing session cleared the slot its successor owns",
        );

        state.release_terminal(&second).await;
        assert!(
            state.terminal().await.is_none(),
            "a session that still owns the slot must be able to release it",
        );
        second.kill();
        first.kill();
    }
}
