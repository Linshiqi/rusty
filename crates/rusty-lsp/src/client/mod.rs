//! The rust-analyzer session.
//!
//! One server per project, spoken to over stdio. The shape mirrors the
//! terminal's: a reader thread feeds an event channel, requests are correlated
//! by id, and the whole thing dies with the child process.
//!
//! What lives here is the session itself — the process, the state its threads
//! share, and how it starts and ends. The messages travel through
//! [`transport`], the `initialize` round trip is [`handshake`], and what the
//! server says unasked is read by [`dispatch`]. The requests are grouped by
//! what they are for: [`documents`], [`edits`], [`query`], [`navigate`] and
//! [`hints`]. Finding the binary is [`discover`], turning replies into the
//! model is [`convert`], keeping diagnostics fresh is [`pull`], and every URI
//! goes through [`uri`].
//!
//! - `check.allTargets` defaults to on, which builds tests and benches. A
//!   `no_std` firmware has no test harness, so that default drowns every real
//!   diagnostic in "can't find crate for `test`". It is turned off.
//! - Diagnostics are **pulled** (LSP 3.17), not just received. After the
//!   build-data workspace switch, r-a never recomputes pushed diagnostics for
//!   open files — they wipe and stay gone. Under pull, it asks the client to
//!   re-request instead, and the puller thread owns freshness.

mod dispatch;
mod documents;
mod edits;
mod handshake;
mod hints;
mod navigate;
mod query;
#[cfg(test)]
mod tests;
mod transport;

use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI64, AtomicU64},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

use self::{
    dispatch::Progress, documents::Doc, edits::Kept, handshake::handshake, transport::pump,
};
use crate::{
    discover,
    error::{Error, Result},
    model::{FileDiagnostic, LspEvent},
    positions::Encoding,
    pull,
};

/// How long `shutdown` may take on the way out. rust-analyzer answers it in
/// milliseconds; one that cannot is about to be killed anyway.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a server told to `exit` is given to do so before `kill`.
const EXIT_GRACE: Duration = Duration::from_millis(500);

/// A running rust-analyzer, and the documents it has been shown.
pub struct LspClient {
    shared: Arc<Shared>,
    /// `None` when the session runs over a transport that is not a child
    /// process — the tests' in-memory pipes.
    child: Mutex<Option<Child>>,
}

/// Diagnostics and lifecycle, for exactly one consumer.
///
/// Ends — `recv` returns `None` — once the client is dropped and the reader
/// has seen the server go: nothing else holds the sender.
pub struct Events {
    rx: Receiver<LspEvent>,
}

impl Events {
    pub fn recv(&self) -> Option<LspEvent> {
        self.rx.recv().ok()
    }

    /// `None` on a timeout *or* on the end of the stream; a caller that has
    /// to tell them apart calls [`Events::recv`] on a thread of its own.
    pub fn recv_timeout(&self, within: Duration) -> Option<LspEvent> {
        self.rx.recv_timeout(within).ok()
    }
}

/// Everything the session's threads share: the writer, the correlation
/// table, the documents, and what the handshake learned.
pub(crate) struct Shared {
    writer: Mutex<Box<dyn Write + Send>>,
    /// Wake the puller for a path. Requests cannot be made from the reader
    /// thread — it would wait on a reply only itself can read — so refreshes
    /// hop threads through this. Set after the handshake, when the loop
    /// starts; the puller holds only a `Weak` to this struct, so dropping
    /// the last strong reference closes the channel and ends it.
    poke: Mutex<Option<Sender<String>>>,
    /// Requests awaiting a reply, by id. `None` down the channel means the
    /// reader is gone and no reply will come.
    pending: Mutex<HashMap<i64, Sender<Option<Value>>>>,
    docs: Mutex<HashMap<String, Doc>>,
    /// The latest completion answers, raw, each with the path it answered
    /// for and its number: the items the frontend sees are converted copies,
    /// and `completionItem/resolve` needs the server's own item — its `data`
    /// in particular — so the accepted one is looked up here by answer and
    /// index.
    completions: Kept,
    /// Numbers the kept answers, completions and code actions alike, from 1
    /// — far short of 2^53 in any session, so it crosses the wire as a plain
    /// JSON number.
    replies: AtomicU64,
    /// The latest code-action answers' WorkspaceEdits, one per fix the
    /// frontend was shown, each with the path and the number it answered
    /// under. An accepted fix that edits other files is applied from here
    /// the way a rename is, since the frontend only ever splices its own
    /// buffer. Kept like the completions and for the same reason: the caret
    /// and a hover both ask, so the newest answer is often not the one the
    /// fix being applied came from.
    actions: Kept,
    /// The server's work in progress, by token — what `$/progress` has begun
    /// and not yet ended. Summarised into `LspEvent::Progress` on change.
    progress: Mutex<BTreeMap<String, Progress>>,
    next_id: AtomicI64,
    /// False once the reader thread has ended: every request from then on
    /// fails at once instead of waiting its budget out for an answer that
    /// cannot arrive.
    alive: AtomicBool,
    /// Set once the handshake has read what the server picked. Diagnostics
    /// only arrive after `initialized`, so the default is never actually used.
    encoding: OnceLock<Encoding>,
    semantic_legend: OnceLock<Vec<String>>,
    /// What each source last said about each file. `pulled` is
    /// rust-analyzer's own analysis, asked for per open document; `pushed`
    /// is what it publishes unasked — with pull negotiated, only the `cargo
    /// check` run. The frontend is always sent the two together
    /// ([`Shared::emit_diagnostics`]): sent as they arrived, each one replaced
    /// the other, and a rustc error showed for three seconds and then an
    /// empty pull took it away.
    pub(crate) pulled: Mutex<HashMap<String, Vec<FileDiagnostic>>>,
    pub(crate) pushed: Mutex<HashMap<String, Vec<FileDiagnostic>>>,
    /// Whether rust-analyzer last said it had nothing left to load. The step
    /// to `true` is when the check runs: a workspace reload clears the
    /// check's results and does not run it again, and opening a project runs
    /// it not at all until something is saved.
    quiescent: AtomicBool,
    /// rust-analyzer registered `workspace/didChangeWatchedFiles`: it is not
    /// watching the disk itself, and hears about changes only from
    /// [`LspClient::did_change_watched_files`] (`watched.rs` says why).
    watching: AtomicBool,
    pub(crate) root: PathBuf,
    pub(crate) events: Sender<LspEvent>,
}

impl LspClient {
    /// Start rust-analyzer for the project at `root`.
    ///
    /// `target` is the triple the firmware builds for, when the caller knows
    /// it — detection does — so cfg resolution matches the chip rather than
    /// the host. `analyzer` names a rust-analyzer to use instead of the one
    /// discovery would pick: every caller passes `None` except the app, which
    /// passes what the user set — see `workbench.toml`'s `rust_analyzer` and
    /// the version skew it exists for.
    pub fn spawn(
        root: &Path,
        target: Option<&str>,
        analyzer: Option<&Path>,
    ) -> Result<(LspClient, Events)> {
        let binary = discover::find_rust_analyzer(analyzer).ok_or(Error::NotFound)?;
        let mut child = discover::command_for(&binary, root)
            .spawn()
            .map_err(Error::Spawn)?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Self::connect(Box::new(stdout), Box::new(stdin), Some(child), root, target)
    }

    /// A session over an already-open transport.
    ///
    /// What [`LspClient::spawn`] builds once the process exists — and what
    /// the tests build over a pair of pipes with a fake server on the other
    /// end, so the handshake, the correlation and the shutdown are proved
    /// against something that answers, without a rust-analyzer on the
    /// machine.
    pub(crate) fn connect(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        child: Option<Child>,
        root: &Path,
        target: Option<&str>,
    ) -> Result<(LspClient, Events)> {
        let (events_tx, events_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            writer: Mutex::new(writer),
            poke: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            docs: Mutex::new(HashMap::new()),
            completions: Kept::default(),
            replies: AtomicU64::new(1),
            actions: Kept::default(),
            progress: Mutex::new(BTreeMap::new()),
            next_id: AtomicI64::new(1),
            alive: AtomicBool::new(true),
            encoding: OnceLock::new(),
            semantic_legend: OnceLock::new(),
            pulled: Mutex::new(HashMap::new()),
            pushed: Mutex::new(HashMap::new()),
            quiescent: AtomicBool::new(false),
            watching: AtomicBool::new(false),
            root: root.to_path_buf(),
            events: events_tx,
        });
        pump(reader, Arc::clone(&shared));

        // The handshake, before anyone else gets the client. A failure here
        // must kill the child by hand — no `LspClient` exists yet to do it on
        // drop, and a leaked rust-analyzer holds the project's target dir open.
        if let Err(e) = handshake(&shared, root, target) {
            if let Some(mut child) = child {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(e);
        }

        let (poke_tx, poke_rx) = mpsc::channel();
        *shared.poke.lock().expect("lsp poke") = Some(poke_tx);
        pull::pull_loop(poke_rx, Arc::downgrade(&shared));

        Ok((
            LspClient {
                shared,
                child: Mutex::new(child),
            },
            Events { rx: events_rx },
        ))
    }
}

impl Drop for LspClient {
    /// Ask before killing. `shutdown` lets rust-analyzer finish what it is
    /// writing and release the target directory; `exit` ends it; the process
    /// is killed only if it lingers. A server that has already died answers
    /// neither, and `alive` makes both return at once rather than after a
    /// timeout — a project switch must not stall on the corpse of the last
    /// server.
    fn drop(&mut self) {
        let _ = self
            .shared
            .request_within("shutdown", Value::Null, SHUTDOWN_TIMEOUT);
        let _ = self.shared.notify("exit", Value::Null);

        let Ok(mut slot) = self.child.lock() else {
            return;
        };
        let Some(mut child) = slot.take() else {
            return;
        };
        let deadline = Instant::now() + EXIT_GRACE;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}
