//! The Crates panel: what the workspace holds, against what crates.io has.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::{CommandPlan, LogLevel, LogLine, LogStream};
use rusty_i18n::t;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Ask crates.io about every direct dependency. Slow by design — one index
/// request per crate — so only the panel's own ask triggers it.
pub fn load_crate_report(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    state.project.crate_rows.set(None);
    track(
        state,
        ipc::get::<Vec<rusty_core::CrateRow>>(cmd::crates::REPORT),
        move |rows| state.project.crate_rows.set(Some(rows)),
    );
}

/// `cargo add name@version` through the shared session slot, then re-analyse
/// — the manifest changed, so the old graph and the old rows are both stale.
pub fn upgrade_crate(state: AppState, name: String, version: String) {
    state.dock.source.set("tools");
    if state.app.session_running.get_untracked() || version.is_empty() {
        return;
    }
    let plan = CommandPlan {
        program: "cargo".to_string(),
        args: vec!["add".to_string(), format!("{name}@{version}")],
        display: format!("cargo add {name}@{version}"),
        rationale: t!("crates.upgrade-rationale"),
        warning: None,
    };
    #[derive(serde::Serialize)]
    struct Args {
        plan: CommandPlan,
    }
    let args = Args { plan };
    let channel = stream_to_terminal(state);
    spawn_local(async move {
        match ipc::call_streaming::<_, Option<i32>>(cmd::flash::RUN, &args, "onLine", &channel)
            .await
        {
            Ok(code) => {
                note_exit(state, code);
                if code == Some(0) {
                    refresh_project(state);
                    load_crate_report(state);
                }
            }
            Err(error) => {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: error.message,
                    level: Some(LogLevel::Error),
                });
                note_exit(state, Some(-1));
            }
        }
    });
}
