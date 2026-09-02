//! The first-run environment check, and the queue that fixes it.
//!
//! A freshly installed workbench on a fresh machine could do nothing until
//! somebody found the Toolchain panel and pressed six buttons in the right
//! order. Everything needed to fix that already existed — the probe, the
//! recipes, the archive downloads — and none of it ran unless asked.
//!
//! So: the check runs itself once the toolchain report lands, and if the
//! machine cannot build, one gesture installs everything in
//! [`rusty_embed::setup::plan`]'s order.
//!
//! **The queue is strictly sequential and stops on the first failure.**
//! Both halves matter. `cargo install espflash` and `espup install` fighting
//! over the same cargo package-cache lock is the failure this repository has
//! already written down once, for Trunk; and a queue that carried on past a
//! failure would end by reporting a machine ready that is not.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::setup::{Destination, SetupStep};

use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Look at the toolchain report and decide whether to interrupt.
///
/// Called when a report lands. Opens the screen only when the machine
/// genuinely cannot build — an optional tool missing is worth *offering*, not
/// worth a dialog — and only once a session, so opening a second project does
/// not reopen a screen somebody has already dismissed.
pub fn check_environment(state: AppState) {
    let Some(report) = state.project.toolchain.get_untracked() else {
        return;
    };
    let steps = rusty_embed::setup::plan(&report);
    let blocked = rusty_embed::setup::blocked(&report);
    state.setup.steps.set(steps);

    if state.setup.checked.get_untracked() {
        return;
    }
    state.setup.checked.set(true);
    if blocked {
        read_data_dir(state);
        state.setup.open.set(true);
    }
}

/// The real path downloads land in.
///
/// Fetched rather than described: "the data directory" is not an answer to
/// "where is this gigabyte going", and this screen is the one place somebody
/// is deciding whether to allow it.
fn read_data_dir(state: AppState) {
    spawn_local(async move {
        if let Ok(location) =
            ipc::get::<rusty_embed::StorageLocation>(cmd::workbench::STORAGE_LOCATION).await
        {
            state.setup.data_dir.set(Some(location.path));
        }
    });
}

/// Open it deliberately — the Help menu's "Check my environment".
///
/// Separate from [`check_environment`] because that one is allowed to decide
/// not to appear, and a menu item that sometimes does nothing is a menu item
/// people stop trusting.
pub fn open_setup(state: AppState) {
    if let Some(report) = state.project.toolchain.get_untracked() {
        state.setup.steps.set(rusty_embed::setup::plan(&report));
    }
    read_data_dir(state);
    state.setup.open.set(true);
}

pub fn close_setup(state: AppState) {
    state.setup.open.set(false);
}

/// Install everything in the plan, in order, stopping at the first failure.
pub fn install_all(state: AppState) {
    if state.setup.running.with_untracked(Option::is_some) {
        return;
    }
    state.setup.installed.update(Vec::clear);
    state.setup.failed.update(Vec::clear);
    state.dock.source.set("tools");
    run_from(state, 0);
}

/// One step, then the next. Recursion rather than a loop because each step is
/// a streaming IPC call that only reports its exit code when it ends.
fn run_from(state: AppState, index: usize) {
    let steps = state.setup.steps.get_untracked();
    let Some(step) = steps.get(index).cloned() else {
        // The queue is done. Re-probe rather than believe it: a step can exit
        // zero and still not put a binary anywhere PATH can see it, and the
        // screen must reflect the machine rather than the exit codes.
        state.setup.running.set(None);
        refresh_toolchain(state);
        return;
    };

    // A manual step is a link, not a command. Nothing to run and nothing to
    // wait for — it is on screen so the user can act on it.
    if step.manual.is_some() {
        state.setup.running.set(None);
        return;
    }

    state.setup.running.set(Some(index));
    let tool = step.tool.clone();

    let finished = move |ok: bool| {
        if ok {
            state.setup.installed.update(|done| done.push(tool.clone()));
            run_from(state, index + 1);
        } else {
            // Stop. Carrying on would end by reporting a ready machine that
            // is not, and the dock already holds the reason this one failed.
            state.setup.failed.update(|bad| bad.push(tool.clone()));
            state.setup.running.set(None);
            refresh_toolchain(state);
        }
    };

    match step.tool.strip_prefix("target:") {
        // `rustup target add` is a plain command; everything else goes
        // through the backend's installer, which knows the multi-step
        // recipes and the archive downloads.
        Some(_) => run_command_then(state, step.command.clone(), move |code| {
            finished(matches!(code, Some(0)));
        }),
        None => install_one(state, step.tool.clone(), finished),
    }
}

/// Stream one tool's installation into the dock and report how it went.
fn install_one(state: AppState, name: String, finished: impl FnOnce(bool) + 'static) {
    #[derive(serde::Serialize)]
    struct Args {
        name: String,
    }

    let channel = stream_to_terminal(state);
    let args = Args { name };
    spawn_local(async move {
        let outcome =
            ipc::call_streaming::<_, Option<i32>>(cmd::sim::INSTALL, &args, "onLine", &channel)
                .await;
        let code = match outcome {
            Ok(code) => code,
            Err(error) => {
                state.app.error.set(Some(error));
                finished(false);
                return;
            }
        };
        note_exit(state, code);
        finished(matches!(code, Some(0)));
    });
}

/// Where a step's output will land, in words, so the screen can say it before
/// anything runs.
pub fn destination_label(step: &SetupStep, data_dir: &str) -> String {
    match step.destination {
        Destination::CargoBin => "~/.cargo/bin — where cargo puts binaries and \
                                  where flashing looks for them"
            .to_string(),
        Destination::RustupHome => "rustup's own directory, with the toolchains".to_string(),
        Destination::DataDirectory => {
            // The separator the path already uses. A Windows path printed
            // with one of each reads as a path somebody assembled by hand,
            // which is exactly what it would be.
            let sep = if data_dir.contains('\\') { '\\' } else { '/' };
            format!("{data_dir}{sep}tools")
        }
        Destination::Manual => "nowhere rusty can reach — this one is yours".to_string(),
    }
}
