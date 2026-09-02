use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use rusty_core::Workspace;
use rusty_embed::catalog::Catalog;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::error::CommandError;

/// Run synchronous work off the async runtime.
///
/// Tauri's async commands share a small pool of worker threads with every
/// other command in flight; a `cargo metadata` or a `probe-rs list` run
/// directly on one of them stalls the rest of the window for its duration —
/// the symptom is every panel freezing while a USB enumeration finishes. The
/// blocking pool is where such work belongs, and `what` names it if it panics.
pub async fn blocking<T, F>(what: &'static str, f: F) -> Result<T, CommandError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CommandError::new(format!("{what} panicked: {e}")))
}

/// What is currently open.
///
/// The Cargo workspace is behind an `Arc` inside the lock so callers can clone
/// the handle and drop the guard immediately. That matters for the assistant: a
/// conversation can run for half a minute, and holding the lock for its
/// duration would freeze every other panel in the window.
///
/// **Every long-lived thing owns a slot here, and a slot is how it ends.** A
/// Tauri channel's `send` fails only when the WebView itself is gone — a
/// JavaScript side that dropped its handler tells the Rust side nothing — so
/// "the user left the panel" is never something a reader loop can notice. The
/// watcher, the language server, the terminal, the debugger, the session and
/// the assistant's question are all ended by whoever replaces or stops the
/// slot, and the loop finds out because its source runs dry.
#[derive(Default)]
pub struct AppState {
    inner: Mutex<Open>,
    /// The debug session, if one is live.
    debugger: Mutex<Option<Arc<rusty_dbg::Debugger>>>,
    /// The flash, monitor or simulator session in flight, if any.
    ///
    /// Only the stopper is kept, not the session: the reader loop blocks on
    /// `recv`, so whoever ends a monitor cannot be the thread sitting inside
    /// it. Behind an `Arc` so a finished reader can release *its own* session
    /// by identity — see [`Self::release_session`].
    session: Mutex<Option<Arc<rusty_embed::process::Stopper>>>,
    /// The running session's stdin, when it has one worth writing — the
    /// simulator's board input path. Cleared with the session.
    session_input: Mutex<Option<rusty_embed::process::Input>>,
    /// The emulator's pin channel, when the running QEMU has rusty's GPIO
    /// model. Present, a button press drives the register the firmware reads;
    /// absent, the board falls back to the `B14=1` message over the UART that
    /// firmware has to be written to expect. Cleared with the session.
    pins: Mutex<Option<crate::simulate::PinChannel>>,
    /// Where a debugger should attach, recorded by the run that armed it.
    attach: Mutex<Option<Attach>>,
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
    /// The project watcher, if one is up, with the ticket its starter holds.
    ///
    /// Numbered rather than shared, because the reader loop must not keep the
    /// watcher alive: the loop ends *because* the watcher was dropped, which
    /// is what closes its receiver. Before this slot existed, every project
    /// switch started a watcher and ended none — one more `notify` handle and
    /// one more thread per switch, the old tree's changes still being pushed
    /// into a window that had moved on.
    watch: Mutex<Option<(u64, rusty_edit::Watch)>>,
    /// The assistant's question in flight: a way to stop it, numbered like the
    /// watcher so a finished question clears only its own entry.
    ///
    /// Without it a question could not be stopped at all — a closed panel
    /// left the loop running for up to eight more tool rounds, on the user's
    /// key.
    asking: Mutex<Option<(u64, CancellationToken)>>,
    /// Serialises every read-modify-write of `workbench.toml` from this
    /// process. See [`Self::update_workbench`].
    workbench: Mutex<()>,
    /// Where the numbered slots get their numbers.
    tickets: AtomicU64,
}

/// What a debug session must attach to, as decided by the run that started it.
///
/// gdb has to read *the ELF that produced the image now running*. This used to
/// be passed up from the frontend, out of the simulation plan it had cached —
/// and that plan was made for a release run. So a debug run booted the
/// unoptimised image while gdb read the optimised binary: every line number
/// came from a different compilation, the breakpoint slid to the next line with
/// code, and its address matched nothing that was executing. One computation,
/// one consumer, and the two cannot drift.
#[derive(Debug, Clone)]
pub struct Attach {
    /// The ELF, relative to the project root — where gdb is started.
    pub elf: String,
    /// The port the target is listening on.
    pub port: u16,
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
    fn ticket(&self) -> u64 {
        self.tickets.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub async fn open(&self, root: PathBuf, workspace: Option<Workspace>) {
        // Reload before taking the other lock: a project's `.rusty/` files are
        // part of what "open" means, and a panel that rendered in between would
        // otherwise see the new root with the old board list. Off the async
        // thread — it reads every overlay file the project and the user have.
        let catalog = {
            let root = root.clone();
            blocking("loading the catalogue", move || {
                Arc::new(Catalog::load(Some(&root)))
            })
            .await
            .unwrap_or_else(|_| Arc::new(Catalog::builtin()))
        };
        *self.catalog.lock().await = Some(catalog);

        let mut guard = self.inner.lock().await;
        guard.root = Some(root);
        guard.workspace = workspace.map(Arc::new);
        // A different project's binary is worse than none.
        guard.firmware = None;
        drop(guard);
        // The old project's server has nothing true left to say, and its
        // watcher reports a tree nobody is looking at.
        self.set_lsp(None).await;
        self.stop_watch().await;
    }

    /// The layered catalogue, falling back to the built-ins before anything is
    /// open — the wizard and the chip list are useful with no project at all.
    pub async fn catalog(&self) -> Arc<Catalog> {
        let mut guard = self.catalog.lock().await;
        if let Some(catalog) = guard.as_ref() {
            return Arc::clone(catalog);
        }
        // Held across the load on purpose: two panels asking at once must not
        // build two catalogues.
        let loaded = blocking("loading the catalogue", || Arc::new(Catalog::load(None)))
            .await
            .unwrap_or_else(|_| Arc::new(Catalog::builtin()));
        *guard = Some(Arc::clone(&loaded));
        loaded
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

    /// Where cargo, espflash and the emulator run.
    ///
    /// The opened directory for every ordinary project. The exception is the
    /// standard embedded workspace — host-testable crates as members, the
    /// bare-metal crate `exclude`d so `cargo test` at the root does not build
    /// `no_std` for the host — where the chip, the target triple and the
    /// toolchain are all one directory down.
    ///
    /// Distinct from [`Self::root`], which stays the directory the user
    /// opened: the file tree, the editor and the language server all belong
    /// to the whole repository. Only the *build* moves.
    pub async fn firmware_root(&self) -> Option<PathBuf> {
        let root = self.root().await?;
        let dir = root.clone();
        blocking("finding the firmware crate", move || {
            rusty_embed::project::firmware_root(&dir)
        })
        .await
        .ok()
        .or(Some(root))
    }

    /// The open project's chip, when detection found one — what picks an
    /// SVD, and what a register view is about.
    pub async fn chip(&self) -> Option<String> {
        let root = self.firmware_root().await?;
        blocking("chip detection", move || {
            rusty_embed::project::detect(&root).ok()?.chip
        })
        .await
        .ok()
        .flatten()
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

    // ─── the project watcher ─────────────────────────────────────────────────

    /// Register the project's watcher, dropping — and thereby stopping —
    /// whatever it replaces. Returns the ticket the starter releases with.
    pub async fn start_watch(&self, watch: rusty_edit::Watch) -> u64 {
        let ticket = self.ticket();
        *self.watch.lock().await = Some((ticket, watch));
        ticket
    }

    /// Release the slot when a watcher's reader ends — only if it still holds
    /// *that* watcher. The reader ends either because the slot already
    /// replaced its watcher (nothing to do) or because the window is gone
    /// (drop it).
    pub async fn release_watch(&self, ticket: u64) {
        let mut slot = self.watch.lock().await;
        if slot.as_ref().is_some_and(|(held, _)| *held == ticket) {
            *slot = None;
        }
    }

    /// Stop whatever watcher is up: a project switch, and exit.
    pub async fn stop_watch(&self) {
        *self.watch.lock().await = None;
    }

    // ─── the assistant's question ────────────────────────────────────────────

    /// Register a question as the one in flight, stopping any it replaces.
    ///
    /// One question at a time is what the panel offers, and the previous one
    /// being still alive underneath a new one would be two loops spending the
    /// same key. Returns the ticket and the token the asker races against.
    pub async fn begin_ask(&self) -> (u64, CancellationToken) {
        let ticket = self.ticket();
        let token = CancellationToken::new();
        let previous = self.asking.lock().await.replace((ticket, token.clone()));
        if let Some((_, previous)) = previous {
            previous.cancel();
        }
        (ticket, token)
    }

    /// Clear the slot when a question finishes — only if it still holds
    /// *that* question.
    pub async fn end_ask(&self, ticket: u64) {
        let mut slot = self.asking.lock().await;
        if slot.as_ref().is_some_and(|(held, _)| *held == ticket) {
            *slot = None;
        }
    }

    /// Stop the question in flight, if any: the panel's Stop, and exit.
    pub async fn cancel_ask(&self) {
        if let Some((_, token)) = self.asking.lock().await.take() {
            token.cancel();
        }
    }

    // ─── workbench.toml ──────────────────────────────────────────────────────

    /// Read, change and write `workbench.toml`, serialised with every other
    /// writer in this process.
    ///
    /// Every setting is a read-modify-write of one file, and two of them
    /// interleaved lose one: the tab strip is recorded on every tab switch,
    /// and a shortcut saved during that window was read back from the file as
    /// it was before the tabs — then written over. One lock, held from the
    /// read to the rename. The IO itself runs on a blocking thread.
    ///
    /// Process-wide, not machine-wide: a second rusty writing the same file
    /// is the storage layer's problem, and `rusty_embed::config` is where a
    /// lock across processes would go. If that layer grows an `update(|s| …)`
    /// of its own, this becomes a call to it.
    pub async fn update_workbench<F>(&self, f: F) -> Result<(), CommandError>
    where
        F: FnOnce(&mut rusty_embed::config::WorkbenchState) + Send + 'static,
    {
        let _serialised = self.workbench.lock().await;
        blocking("saving workbench.toml", move || {
            let mut state = rusty_embed::config::workbench();
            f(&mut state);
            rusty_embed::config::save_workbench(&state)
        })
        .await?
        .map_err(CommandError::from)
    }

    /// Run one of the storage layer's own read-modify-writes of
    /// `workbench.toml` — recents, tabs — under the same lock as
    /// [`Self::update_workbench`], so they cannot interleave with a setting.
    pub async fn with_workbench<T, F>(&self, what: &'static str, f: F) -> Result<T, CommandError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let _serialised = self.workbench.lock().await;
        blocking(what, f).await
    }

    // ─── the running session ─────────────────────────────────────────────────

    pub async fn set_firmware(&self, path: Option<PathBuf>) {
        self.inner.lock().await.firmware = path;
    }

    pub async fn set_session_input(&self, input: Option<rusty_embed::process::Input>) {
        *self.session_input.lock().await = input;
    }

    pub async fn session_input(&self) -> Option<rusty_embed::process::Input> {
        self.session_input.lock().await.clone()
    }

    pub async fn set_pins(&self, pins: Option<crate::simulate::PinChannel>) {
        *self.pins.lock().await = pins;
    }

    pub async fn pins(&self) -> Option<crate::simulate::PinChannel> {
        self.pins.lock().await.clone()
    }

    /// Record where the debugger should attach. Set by the run that built the
    /// image and armed the target, because that run is the only thing that
    /// knows which binary is now executing.
    pub async fn set_attach(&self, attach: Option<Attach>) {
        *self.attach.lock().await = attach;
    }

    pub async fn attach(&self) -> Option<Attach> {
        self.attach.lock().await.clone()
    }

    /// Register a running session, ending any previous one.
    ///
    /// Two monitors on the same serial port cannot both work — the second gets
    /// an access-denied that reads like a driver problem — so starting one
    /// always stops the last. Returns the handle the starter's reader releases
    /// the slot with, by identity.
    pub async fn start_session(
        &self,
        stopper: rusty_embed::process::Stopper,
    ) -> Arc<rusty_embed::process::Stopper> {
        let stopper = Arc::new(stopper);
        let previous = self.session.lock().await.replace(Arc::clone(&stopper));
        if let Some(previous) = previous {
            previous.stop();
        }
        stopper
    }

    /// Release the slot when a session's reader ends — only if it still holds
    /// *that* session.
    ///
    /// The same bug the terminal and the debugger had, in the third slot. A
    /// finished reader used to `stop_session()`, which takes and stops
    /// whatever is there: start B while A runs, A is killed and its reader
    /// ends, and A's cleanup then killed B. The input and the pin channel
    /// belong to the session, so they go with it — and only with it.
    pub async fn release_session(&self, ours: &Arc<rusty_embed::process::Stopper>) {
        let mut slot = self.session.lock().await;
        if slot.as_ref().is_some_and(|held| Arc::ptr_eq(held, ours)) {
            *slot = None;
            *self.session_input.lock().await = None;
            *self.pins.lock().await = None;
        }
    }

    /// Stop whatever session is running: the user's Stop, and exit.
    ///
    /// Only these two callers may say a session ended without owning it. A
    /// reader that has finished releases its own — see
    /// [`Self::release_session`].
    pub async fn stop_session(&self) {
        // Same lock order as `release_session`: session, then what it owns.
        let stopper = self.session.lock().await.take();
        *self.session_input.lock().await = None;
        // With the emulator gone the pin channel is a socket to nothing. Left
        // behind, the next button press would be written into it and vanish,
        // which reads as firmware ignoring the press.
        *self.pins.lock().await = None;
        if let Some(stopper) = stopper {
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
        let firmware = match open.firmware.clone() {
            Some(firmware) => Some(firmware),
            None => {
                let root = open.root.clone();
                blocking("firmware discovery", move || {
                    let root = root?;
                    let configured = rusty_embed::project::detect(&root)
                        .ok()
                        .and_then(|p| p.configured_target);
                    rusty_embed::firmware::newest(&root, configured.as_deref())
                        .map(|f| PathBuf::from(f.path))
                })
                .await
                .ok()
                .flatten()
            }
        };

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
    use std::sync::atomic::AtomicBool;

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

        let held = state
            .terminal()
            .await
            .expect("the new session is still open");
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

    /// A stopper that only records being stopped — the session slot holds
    /// stoppers, not processes, so no process is needed to test its rules.
    fn flagged() -> (rusty_embed::process::Stopper, Arc<AtomicBool>) {
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        let stopper =
            rusty_embed::process::Stopper::new(move || flag.store(true, Ordering::SeqCst));
        (stopper, stopped)
    }

    /// The same bug as the terminal's, in the session slot: while A runs, the
    /// user starts B, which kills A; A's reader then ends and — before this —
    /// called `stop_session()`, killing B. The Stop button vanished while
    /// QEMU kept running with nothing in the window able to reach it.
    #[tokio::test]
    async fn a_finished_session_reader_releases_only_its_own_session() {
        let state = AppState::default();
        let (a, a_stopped) = flagged();
        let (b, b_stopped) = flagged();

        let ours_a = state.start_session(a).await;
        let ours_b = state.start_session(b).await;
        assert!(
            a_stopped.load(Ordering::SeqCst),
            "starting B stops A — one session at a time"
        );
        assert!(!b_stopped.load(Ordering::SeqCst));

        // A's reader notices A is gone and cleans up after itself.
        state.release_session(&ours_a).await;
        assert!(
            !b_stopped.load(Ordering::SeqCst),
            "A's cleanup must not stop the session that replaced it",
        );
        assert!(
            state.session.lock().await.is_some(),
            "and must not clear the slot B owns",
        );

        // B's own reader ending releases the slot.
        state.release_session(&ours_b).await;
        assert!(state.session.lock().await.is_none());
        assert!(
            !b_stopped.load(Ordering::SeqCst),
            "releasing is not stopping: B ended on its own",
        );

        // The user's Stop is the one unconditional path.
        let (c, c_stopped) = flagged();
        let _ = state.start_session(c).await;
        state.stop_session().await;
        assert!(c_stopped.load(Ordering::SeqCst));
        assert!(state.session.lock().await.is_none());
    }

    /// The watcher leak: every `watch_project` started a watcher and nothing
    /// ended it. With a slot, replacing a watcher drops it, and dropping it is
    /// what ends its reader — the receiver closes. And, like every other slot,
    /// the outgoing reader must not clear the entry its successor owns.
    #[tokio::test]
    async fn replacing_the_watcher_ends_the_old_reader_and_keeps_the_new_slot() {
        let dir = tempfile::tempdir().expect("a directory to watch");
        let state = AppState::default();

        let (first, first_changes) = rusty_edit::watch(dir.path()).expect("a watcher");
        let first_ticket = state.start_watch(first).await;

        let (second, _second_changes) = rusty_edit::watch(dir.path()).expect("a watcher");
        let second_ticket = state.start_watch(second).await;

        // The first watcher is gone, so its reader's `recv()` must return —
        // this is the contract the leak fix leans on.
        let ended = first_changes
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_err();
        assert!(ended, "dropping a Watch has to close its receiver");

        // The first reader, ending, releases by ticket: nothing happens.
        state.release_watch(first_ticket).await;
        assert!(
            state.watch.lock().await.is_some(),
            "the outgoing reader cleared the slot its successor owns",
        );

        state.release_watch(second_ticket).await;
        assert!(state.watch.lock().await.is_none());
    }

    /// A question supersedes the one before it, an explicit cancel stops the
    /// one in flight, and a finished question clears only its own entry.
    #[tokio::test]
    async fn a_new_question_supersedes_the_last_and_only_its_own_end_clears_the_slot() {
        let state = AppState::default();

        let (first_ticket, first) = state.begin_ask().await;
        let (second_ticket, second) = state.begin_ask().await;
        assert!(first.is_cancelled(), "the earlier question is stopped");
        assert!(!second.is_cancelled());

        state.end_ask(first_ticket).await;
        assert!(
            state.asking.lock().await.is_some(),
            "the earlier question finishing must not clear the later one",
        );

        state.cancel_ask().await;
        assert!(second.is_cancelled(), "Stop stops the question in flight");
        assert!(state.asking.lock().await.is_none());

        // Cancelling with nothing in flight is a no-op, and a question that
        // ends normally clears itself.
        state.cancel_ask().await;
        let (third_ticket, third) = state.begin_ask().await;
        state.end_ask(third_ticket).await;
        assert!(!third.is_cancelled());
        assert!(state.asking.lock().await.is_none());
        let _ = second_ticket;
    }
}
