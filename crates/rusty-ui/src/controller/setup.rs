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
use rusty_i18n::t;

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

// The deliberate way in — Help ▸ "Check my environment" — is the
// Environment page now, which says everything this sheet does and what is
// installed besides; the sheet stays what interrupts a machine that cannot
// build.

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
    run_step(state, step, move |ok| {
        if ok {
            run_from(state, index + 1);
        } else {
            // Stop. Carrying on would end by reporting a ready machine that
            // is not, and the dock already holds the reason this one failed.
            state.setup.running.set(None);
            refresh_toolchain(state);
        }
    });
}

/// The Environment page's "Install what is missing": the steps for exactly
/// the tools it marked as needed, in the plan's order, stopping at the first
/// failure as the sheet's queue does. Not the sheet's whole plan — that one
/// offers the optional tools too, which is right on a first run and wrong
/// under a button that says what it installs.
pub fn install_needed(state: AppState, tools: Vec<String>) {
    if state.app.session_running.get_untracked() || state.setup.busy.with_untracked(Option::is_some)
    {
        return;
    }
    let Some(report) = state.project.toolchain.get_untracked() else {
        return;
    };
    let steps: Vec<SetupStep> = rusty_embed::setup::plan(&report)
        .into_iter()
        .filter(|step| step.manual.is_none() && tools.contains(&step.tool))
        .collect();
    state.dock.source.set("tools");
    run_list(state, steps, 0);
}

fn run_list(state: AppState, steps: Vec<SetupStep>, index: usize) {
    let Some(step) = steps.get(index).cloned() else {
        refresh_toolchain(state);
        return;
    };
    run_step(state, step, move |ok| {
        if ok {
            run_list(state, steps, index + 1);
        } else {
            refresh_toolchain(state);
        }
    });
}

/// One row's Install on the Environment page: the step the queue would run
/// for that tool, run alone, with the same progress the queue shows.
pub fn install_step(state: AppState, tool: String) {
    if state.app.session_running.get_untracked() || state.setup.busy.with_untracked(Option::is_some)
    {
        return;
    }
    let Some(report) = state.project.toolchain.get_untracked() else {
        return;
    };
    let Some(step) = rusty_embed::setup::plan(&report)
        .into_iter()
        .find(|step| step.tool == tool && step.manual.is_none())
    else {
        return;
    };
    state.dock.source.set("tools");
    run_step(state, step, move |_| refresh_toolchain(state));
}

/// One step, and the progress every screen that shows installs reads: busy
/// while it runs, then installed or failed.
fn run_step(state: AppState, step: SetupStep, finished: impl FnOnce(bool) + 'static) {
    let tool = step.tool.clone();
    state.setup.busy.set(Some(tool.clone()));
    state.setup.failed.update(|bad| bad.retain(|t| t != &tool));
    let done = move |ok: bool| {
        state.setup.busy.set(None);
        let list = if ok {
            state.setup.installed
        } else {
            state.setup.failed
        };
        list.update(|names| {
            if !names.contains(&tool) {
                names.push(tool.clone());
            }
        });
        // The editor gets its language server the moment it exists, rather
        // than at the next project open.
        if ok && tool == "rust-analyzer" {
            start_lsp(state);
        }
        finished(ok);
    };

    match step.tool.strip_prefix("target:") {
        // `rustup target add` is a plain command; everything else goes
        // through the backend's installer, which knows the multi-step
        // recipes and the archive downloads. Both are installs to the
        // status bar, named by what they install.
        Some(target) => {
            let target = target.to_string();
            run_command_on(state, step.command.clone(), "tools", move |code| {
                done(matches!(code, Some(0)));
            });
            name_activity(state, target);
        }
        None => install_one(state, step.tool.clone(), done),
    }
}

/// Stream one tool's installation into the dock and report how it went.
fn install_one(state: AppState, name: String, finished: impl FnOnce(bool) + 'static) {
    #[derive(serde::Serialize)]
    struct Args {
        name: String,
    }

    let channel = stream_to_terminal(state);
    name_activity(state, name.clone());
    let args = Args { name };
    spawn_local(async move {
        let outcome =
            ipc::call_streaming::<_, Option<i32>>(cmd::sim::INSTALL, &args, "onLine", &channel)
                .await;
        let code = match outcome {
            Ok(code) => code,
            Err(error) => {
                // The session this started has ended too: without the exit
                // the Stop button stayed up over nothing.
                state.app.error.set(Some(error));
                note_exit(state, Some(-1));
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
        Destination::CargoBin => t!("setup.where.cargo-bin"),
        Destination::RustupHome => t!("setup.where.rustup-home"),
        Destination::DataDirectory => {
            // The separator the path already uses. A Windows path printed
            // with one of each reads as a path somebody assembled by hand,
            // which is exactly what it would be.
            let sep = if data_dir.contains('\\') { '\\' } else { '/' };
            format!("{data_dir}{sep}tools")
        }
        Destination::Manual => t!("setup.where.manual"),
    }
}
