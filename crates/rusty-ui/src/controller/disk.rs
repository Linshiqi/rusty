//! The Disk section: what the build directory holds, and the removals it
//! offers.
//!
//! Every removal names a policy or a path the backend re-derives itself —
//! the frontend never sends a list of files to delete. Whole trees and
//! caches ask first, in words that carry the size; a sweep of stale
//! artifacts does not, because the table above the button is the preview
//! and nothing a sweep removes is needed by the build as configured.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_core::{DiskReport, SweepPolicy, SweepReport};
use rusty_embed::{LogLevel, LogLine, LogStream};
use rusty_i18n::t;

use crate::{
    format,
    ipc::{self, cmd},
    state::AppState,
};

/// Measure the build directory. Slow on a big one — a scan reads every
/// dep-info file — so only the section's own ask triggers it.
pub fn load_disk_report(state: AppState) {
    if !state.has_project_now() || state.project.disk_busy.get_untracked() {
        return;
    }
    state.project.disk_busy.set(true);
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        idle_days: u32,
    }
    let args = Args {
        idle_days: state.project.disk_idle_days.get_untracked(),
    };
    spawn_local(async move {
        match ipc::call::<_, DiskReport>(cmd::disk::REPORT, &args).await {
            Ok(report) => state.project.disk.set(Some(report)),
            Err(error) => report_failure(state, error),
        }
        state.project.disk_busy.set(false);
    });
}

/// Remove the stale artifacts of one tree, or of every tree, then measure
/// again.
pub fn sweep_disk(state: AppState, tree: Option<String>) {
    let policy = SweepPolicy {
        tree,
        idle_days: Some(state.project.disk_idle_days.get_untracked()),
        ..SweepPolicy::default()
    };
    #[derive(serde::Serialize)]
    struct Args {
        policy: SweepPolicy,
    }
    run_removal(state, async move {
        ipc::call::<_, SweepReport>(cmd::disk::SWEEP, &Args { policy }).await
    });
}

/// Remove a whole build tree or a known extra — after asking, with the size.
pub fn remove_disk_path(state: AppState, path: String, bytes: u64) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    let question = t!(
        "disk.remove-confirm",
        path = path.clone(),
        size = format::bytes(bytes)
    );
    spawn_local(async move {
        if !ipc::confirm(&question).await {
            return;
        }
        run_removal(state, async move {
            ipc::call::<_, SweepReport>(cmd::disk::REMOVE, &Args { path }).await
        });
    });
}

/// Remove one of cargo's caches — after asking, since the cost is a
/// download or an extraction the next build pays.
pub fn remove_disk_cache(state: AppState, label: String, bytes: u64) {
    #[derive(serde::Serialize)]
    struct Args {
        label: String,
    }
    let question = t!(
        "disk.cache-confirm",
        name =
            rusty_i18n::translate(&format!("disk.cache.{label}")).unwrap_or_else(|| label.clone()),
        size = format::bytes(bytes)
    );
    spawn_local(async move {
        if !ipc::confirm(&question).await {
            return;
        }
        run_removal(state, async move {
            ipc::call::<_, SweepReport>(cmd::disk::REMOVE_CACHE, &Args { label }).await
        });
    });
}

fn run_removal(
    state: AppState,
    call: impl std::future::Future<Output = ipc::Answer<SweepReport>> + 'static,
) {
    if state.project.disk_busy.get_untracked() {
        return;
    }
    state.project.disk_busy.set(true);
    state.dock.source.set("tools");
    spawn_local(async move {
        match call.await {
            Ok(report) => {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: t!(
                        "disk.swept",
                        count = report.removed_items.to_string(),
                        size = format::bytes(report.removed_bytes)
                    ),
                    level: None,
                });
                for tree in &report.locked {
                    state.push_log(LogLine {
                        stream: LogStream::Stderr,
                        text: t!("disk.skipped-locked", path = tree.clone()),
                        level: Some(LogLevel::Warn),
                    });
                }
                for failed in &report.failed {
                    state.push_log(LogLine {
                        stream: LogStream::Stderr,
                        text: failed.clone(),
                        level: Some(LogLevel::Error),
                    });
                }
            }
            Err(error) => report_failure(state, error),
        }
        state.project.disk_busy.set(false);
        load_disk_report(state);
    });
}

fn report_failure(state: AppState, error: ipc::IpcError) {
    state.push_log(LogLine {
        stream: LogStream::Stderr,
        text: error.message.clone(),
        level: Some(LogLevel::Error),
    });
    state.app.error.set(Some(error));
}

/// The auto-sweep switch, read from `workbench.toml` when the section mounts
/// and written back when toggled — a file, because the backend reads it at
/// the end of every build and a second window has to agree.
pub fn load_disk_auto_sweep(state: AppState) {
    spawn_local(async move {
        if let Ok(on) = ipc::get::<bool>(cmd::disk::AUTO_SWEEP).await {
            state.project.disk_auto_sweep.set(on);
        }
    });
}

pub fn set_disk_auto_sweep(state: AppState, enabled: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        enabled: bool,
    }
    state.project.disk_auto_sweep.set(enabled);
    spawn_local(async move {
        if let Err(error) = ipc::call::<_, ()>(cmd::disk::SET_AUTO_SWEEP, &Args { enabled }).await {
            report_failure(state, error);
        }
    });
}

/// A new idle threshold: remembered for the session and measured against at
/// once, so the stale figures answer the question just asked.
pub fn set_disk_idle_days(state: AppState, days: u32) {
    state.project.disk_idle_days.set(days);
    load_disk_report(state);
}

/// Remove every incremental cache of one tree — the biggest single item on
/// a hot workspace, and safe: rustc rebuilds a cache from nothing at the
/// cost of one slower compile per crate. Asks first, with the size.
pub fn drop_incremental(state: AppState, tree: String, bytes: u64) {
    let separator = if tree.contains('\\') { "\\" } else { "/" };
    let path = format!("{tree}{separator}incremental");
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    let question = t!("disk.incremental-confirm", size = format::bytes(bytes));
    spawn_local(async move {
        if !ipc::confirm(&question).await {
            return;
        }
        run_removal(state, async move {
            ipc::call::<_, SweepReport>(cmd::disk::REMOVE, &Args { path }).await
        });
    });
}
