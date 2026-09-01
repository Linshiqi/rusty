//! The only place a cross-layer action begins.
//!
//! Views render; controllers fetch, mutate, and record failure. Keeping that
//! split means the busy indicator, the error surface, and the ordering rules
//! are written once instead of being re-invented per panel.
//!
//! One module per thing the workbench does. It was one file of 3,800 lines
//! and 146 functions, already sectioned by hand-drawn dividers — those
//! dividers were module boundaries that had never been made into modules, so
//! nothing stopped the debugger's section from accreting the editor, the
//! update check and the keybinds. The names below are those dividers.
//!
//! Everything is re-exported flat, so a caller still writes
//! `controller::open_file(…)` and never has to know which module it moved to.
//! The submodules are how *this* code is organised, not a second vocabulary
//! for the views to learn.

mod assistant;
mod crates;
mod debug;
mod devices;
mod editor;
mod lsp;
mod navigate;
mod project;
mod session;
mod simulate;
mod storage;
mod terminal;
mod watch;
mod wizard;
mod workbench;

pub use assistant::*;
pub use crates::*;
pub use debug::*;
pub use devices::*;
pub use editor::*;
pub use lsp::*;
pub use navigate::*;
pub use project::*;
pub use session::*;
pub use simulate::*;
pub use storage::*;
pub use terminal::*;
pub use watch::*;
pub use wizard::*;
pub use workbench::*;

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_core::WorkspaceReport;
use rusty_embed::{EmbeddedProject, LogLevel, LogLine, LogStream};

use crate::{ipc::Answer, state::AppState};

/// What `open_project` returns. Mirrors `rusty_app::commands::OpenResult`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenResult {
    project: EmbeddedProject,
    workspace: Option<WorkspaceReport>,
    /// Why the Cargo analysis is absent, when it is.
    ///
    /// Surfaced rather than dropped. Opening succeeds either way — a project
    /// whose `cargo metadata` fails is exactly the one whose diagnosis matters
    /// — but the panels that go empty because of it cannot explain themselves,
    /// so the reason goes to the dock where it stays answerable.
    workspace_error: Option<String>,
}

/// Run an action, tracking it as in flight and recording any failure.
///
/// Every controller entry point goes through this, so a panel can never leave
/// the spinner spinning by forgetting to decrement it on the error path.
fn track<F, T>(state: AppState, future: F, apply: impl FnOnce(T) + 'static)
where
    F: std::future::Future<Output = Answer<T>> + 'static,
    T: 'static,
{
    tracked(state, future, apply, false);
}

/// [`track`] for a call that *started* the session: its failure is that
/// session ending, so the Stop button and the terminal's prompt come back.
///
/// The distinction is load-bearing because `session_running` is one global
/// flag, and `track` wraps *every* controller entry point. Clearing it on any
/// failure meant one unrelated error — a register read refused because the
/// debugger had stopped, a workspace query that could not answer — told a
/// running simulation it had ended. Its Stop button vanished while QEMU kept
/// going, leaving a process the window could no longer stop.
fn track_session<F, T>(state: AppState, future: F, apply: impl FnOnce(T) + 'static)
where
    F: std::future::Future<Output = Answer<T>> + 'static,
    T: 'static,
{
    tracked(state, future, apply, true);
}

fn tracked<F, T>(state: AppState, future: F, apply: impl FnOnce(T) + 'static, owns_session: bool)
where
    F: std::future::Future<Output = Answer<T>> + 'static,
    T: 'static,
{
    state.app.in_flight.update(|n| *n += 1);
    spawn_local(async move {
        match future.await {
            Ok(value) => {
                state.app.error.set(None);
                apply(value);
            }
            Err(e) => {
                if owns_session {
                    // A session that failed to start is a session that is not
                    // running — otherwise a tool that is not installed leaves
                    // the Stop button up and the prompt refusing to send.
                    state.app.session_running.set(false);
                }
                // The banner is transient — dismissed, or replaced by the next
                // failure. The dock keeps it, so "what did that error say?" is
                // answerable after the fact.
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: e.message.clone(),
                    level: Some(LogLevel::Error),
                });
                for cause in &e.causes {
                    state.push_log(LogLine {
                        stream: LogStream::Stderr,
                        text: format!("  {cause}"),
                        level: Some(LogLevel::Error),
                    });
                }
                state.app.error.set(Some(e));
            }
        }
        state.app.in_flight.update(|n| *n = n.saturating_sub(1));
    });
}
