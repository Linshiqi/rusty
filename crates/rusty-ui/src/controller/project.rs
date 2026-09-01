//! Opening a project, and everything that follows from it.
//!
//! One place decides what "open" means — detection, the Cargo analysis, the
//! catalogue, the tab strip — because the picker path and the recents path
//! both go through it, and drifting apart is how one of them forgets a step.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_core::{FeatureImpact, FeatureRow, FeatureSelection, WorkspaceReport};
use rusty_embed::{
    Board, Chip, EmbeddedProject, Firmware, LogLevel, LogLine, LogStream, MemoryReport, Migration,
    ToolchainReport,
};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, EditHistory},
};

pub fn load_catalog(state: AppState) {
    track(
        state,
        ipc::get::<Vec<Chip>>(cmd::catalog::CHIPS),
        move |chips| state.project.chips.set(chips),
    );
    track(
        state,
        ipc::get::<Vec<Board>>(cmd::catalog::BOARDS),
        move |boards| state.project.boards.set(boards),
    );
}

pub fn load_recents(state: AppState) {
    spawn_local(async move {
        if let Ok(list) = ipc::get::<Vec<String>>(cmd::workbench::RECENTS).await {
            state.app.recents.set(list);
        }
    });
}

/// Open a project from the recents list.
///
/// `announce` decides what failure looks like. Clicked by a human: a banner.
/// Tried automatically at launch: a log line — a red banner as the first thing
/// every morning because a folder moved would train people to ignore banners —
/// and either way the stale entry is forgotten so it stops being offered.
pub fn open_recent(state: AppState, path: String, announce: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    #[derive(serde::Serialize)]
    struct Forget {
        path: String,
    }

    let args = Args { path: path.clone() };
    spawn_local(async move {
        match ipc::call::<_, OpenResult>(cmd::project::OPEN, &args).await {
            Ok(result) => {
                project_opened(state, result);
                load_recents(state);
                load_keybinds(state);
                apply_ui_zoom(state);
            }
            Err(error) => {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("{path} could not be reopened: {}", error.message),
                    level: Some(LogLevel::Warn),
                });
                let _ = ipc::call::<_, ()>(
                    cmd::workbench::FORGET_RECENT,
                    &Forget { path: path.clone() },
                )
                .await;
                load_recents(state);
                if announce {
                    state.app.error.set(Some(error));
                }
            }
        }
    });
}

/// Ask the OS for a folder, then open it.
///
/// Cancelling is not a failure and must not surface as one — which is why this
/// does not go through `track`.
pub fn choose_project(state: AppState) {
    spawn_local(async move {
        match ipc::pick_folder("Open a Cargo project").await {
            Ok(Some(path)) => open_project(state, path),
            Ok(None) => {}
            Err(e) => state.app.error.set(Some(e)),
        }
    });
}

/// Open a folder as the project.
pub fn open_project(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    // Bound rather than passed as a temporary: the future outlives this
    // statement, so a borrow of an inline struct literal would dangle.
    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, OpenResult>(cmd::project::OPEN, &args).await },
        move |result| {
            project_opened(state, result);
            load_recents(state);
        },
    );
}

/// Everything that follows a successful open, shared by the picker path and
/// the recents path so the two cannot drift apart.
fn project_opened(state: AppState, result: OpenResult) {
    {
        {
            state.project.detected.set(Some(result.project));
            state.project.workspace.set(result.workspace);
            if let Some(detail) = result.workspace_error {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("cargo metadata is unavailable: {detail}"),
                    level: Some(LogLevel::Warn),
                });
            }
            // Another project's binary is worse than none — it would be analysed
            // against this project's chip and report plausible nonsense.
            state.project.selected_firmware.set(None);
            state.project.memory.set(None);
            state.editor.document.set(None);
            state.editor.draft.set(String::new());
            state.editor.tabs.set(Vec::new());
            state.editor.parked.set(Vec::new());
            state.editor.history.set(EditHistory::default());
            state.editor.completion.set(None);
            state.editor.signature.set(None);
            state.search.query.set(String::new());
            state.search.results.set(None);
            state.search.word.set(false);
            state.find.open.set(false);
            state.find.replace_open.set(false);
            state.find.query.set(String::new());
            state.find.index.set(0);
            state.search.regex.set(false);
            state.search.include.set(String::new());
            state.search.exclude.set(String::new());
            state.editor.tree.set(Vec::new());
            state.editor.expanded.set(Vec::new());
            state.editor.highlighted.set(Vec::new());
            state.editor.echo_text.set(String::new());
            state.lsp.diagnostics.set(std::collections::HashMap::new());
            state.editor.hover.set(None);
            state.editor.reveal.set(None);
            // The selection names a member of the *previous* workspace, so
            // keeping it would ask the backend to resolve features for a package
            // that is not there.
            state.project.feature_selection.set(None);
            state.project.feature_rows.set(Vec::new());
            state.project.feature_impact.set(None);
            // The toolchain verdict depends on which chip was just detected, so
            // it has to follow rather than run alongside.
            refresh_toolchain(state);
            refresh_firmware(state);
            refresh_tree(state);
            start_lsp(state);
            start_watch(state);
            if let Some(root) = state
                .project
                .detected
                .with_untracked(|p| p.as_ref().map(|p| p.root.clone()))
            {
                restore_tabs(state, &root);
            }
        }
    }
}

/// Re-read the project's files without reopening it.
///
/// Guarded, because this is what the toolbar button and `Ctrl R` reach: asking
/// the backend to re-check nothing produces an error banner about no project
/// being open, which the user can see for themselves.
pub fn refresh_project(state: AppState) {
    if state.has_project() {
        reload_project(state);
    }
}

/// Fetch the project the backend holds, whether or not this window knows about
/// it yet.
///
/// Separate from [`refresh_project`] because [`restore`] runs when the frontend
/// has *no* project and the backend does — the guard above would reject exactly
/// the case restore exists to handle, and a reload would silently drop the open
/// project with nothing on screen to explain it.
fn reload_project(state: AppState) {
    track(
        state,
        ipc::get::<EmbeddedProject>(cmd::project::STATUS),
        move |project| {
            let root = project.root.clone();
            state.project.detected.set(Some(project));
            refresh_toolchain(state);
            refresh_firmware(state);
            refresh_workspace(state);
            refresh_tree(state);
            start_lsp(state);
            start_watch(state);
            // A WebView reload reaches a project the backend never closed —
            // this path skips project_opened, so the strip is replayed here
            // too or a refresh would silently drop every open tab.
            restore_tabs(state, &root);
        },
    );
}

/// Re-fetch the Cargo analysis.
///
/// Deliberately not tracked. A workspace whose `cargo metadata` failed is a
/// normal state for a misconfigured embedded project — the case this workbench
/// is *for* — and the panels that need it already say so in their own terms. A
/// red banner on every restore would be crying wolf about the expected thing.
fn refresh_workspace(state: AppState) {
    spawn_local(async move {
        match ipc::get::<WorkspaceReport>(cmd::project::WORKSPACE_REPORT).await {
            Ok(report) => state.project.workspace.set(Some(report)),
            Err(e) => {
                state.project.workspace.set(None);
                // Kept in the dock rather than dropped: "why is the Features
                // panel empty" has to be answerable after the fact.
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("cargo metadata is unavailable: {}", e.message),
                    level: Some(LogLevel::Warn),
                });
            }
        }
    });
}

/// Re-scan the target directory for built binaries.
///
/// Cheap enough to run on every project refresh: it is a directory walk and a
/// four-byte read per candidate, and a memory panel showing yesterday's build
/// list is worse than useless — it is confidently wrong.
pub fn refresh_firmware(state: AppState) {
    if !state.has_project() {
        return;
    }
    track(
        state,
        ipc::get::<Vec<Firmware>>(cmd::firmware::LIST),
        move |found| {
            // Drop a selection that no longer exists, so the panel falls back to
            // the default rather than showing a path that was just cleaned away.
            state.project.selected_firmware.update(|selected| {
                if let Some(path) = selected.as_deref()
                    && !found.iter().any(|f| f.path == path)
                {
                    *selected = None;
                }
            });
            state.project.firmware.set(found);
        },
    );
}

/// Analyse a built ELF.
pub fn analyze_memory(state: AppState, elf_path: String) {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        elf_path: String,
    }

    let args = Args { elf_path };
    track(
        state,
        async move { ipc::call::<_, MemoryReport>(cmd::memory::REPORT, &args).await },
        move |report| state.project.memory.set(Some(report)),
    );
}

/// Simulate a feature selection, updating both halves of the matrix.
///
/// The rows and the impact are two queries because they answer two questions —
/// what each switch costs on its own, and what the whole selection costs — and
/// they are issued together because a screen showing one recomputed and the
/// other stale is worse than a screen showing neither.
pub fn apply_features(state: AppState, selection: FeatureSelection) {
    #[derive(serde::Serialize)]
    struct Args {
        selection: FeatureSelection,
    }

    state.project.feature_selection.set(Some(selection.clone()));

    let rows_args = Args {
        selection: selection.clone(),
    };
    track(
        state,
        async move { ipc::call::<_, Vec<FeatureRow>>(cmd::features::ROWS, &rows_args).await },
        move |rows| state.project.feature_rows.set(rows),
    );

    let impact_args = Args { selection };
    track(
        state,
        async move { ipc::call::<_, FeatureImpact>(cmd::features::IMPACT, &impact_args).await },
        move |impact| state.project.feature_impact.set(Some(impact)),
    );
}

pub fn refresh_toolchain(state: AppState) {
    track(
        state,
        ipc::get::<ToolchainReport>(cmd::toolchain::REPORT),
        move |report| state.project.toolchain.set(Some(report)),
    );
}

/// Reattach to a project the backend still holds, after a frontend reload.
pub fn restore(state: AppState) {
    if !ipc::backend_available() {
        state.app.error.set(Some(ipc::IpcError {
            message: "Running outside Tauri, so nothing can be loaded.".into(),
            causes: vec![
                "This is the Trunk dev server in a plain browser. The layout and \
                 styling are real; anything that needs the backend is not."
                    .into(),
                "Run `cargo tauri dev` from crates/rusty-app for the whole app.".into(),
            ],
        }));
        return;
    }

    // Before anything else, because it decides what the keyboard means. Not
    // tied to opening a project: someone who edits in Vim keys wants them in
    // the window that is already open, and in the next one.
    load_vim(state);

    // Neither the catalogue nor the machine's toolchain depends on a project,
    // so both load unconditionally. Sequencing them after the project probe
    // would mean one failed probe leaving those panels empty for the whole
    // session, with nothing on screen to say why.
    load_catalog(state);
    load_provider(state);
    refresh_toolchain(state);

    load_recents(state);

    spawn_local(async move {
        // Nothing open is the normal cold-start state, not a failure worth
        // showing, so this one deliberately does not go through `track`.
        if let Ok(Some(_path)) = ipc::get::<Option<String>>(cmd::project::PATH).await {
            reload_project(state);
            return;
        }
        // A fresh launch: pick up where the last session left off. Quietly —
        // a moved folder degrades to the normal empty state plus a log line,
        // and stops being offered.
        if let Ok(list) = ipc::get::<Vec<String>>(cmd::workbench::RECENTS).await
            && let Some(last) = list.first()
        {
            open_recent(state, last.clone(), false);
        }
    });
}

// ─── chips ───────────────────────────────────────────────────────────────────

/// The part's pins and what the source names, for the editor's pin map.
pub fn load_pin_report(state: AppState) {
    track(
        state,
        ipc::get::<Option<rusty_embed::PinReport>>(cmd::pins::REPORT),
        move |report| state.project.pins.set(report),
    );
}

/// What switching this project to another chip would change.
pub fn plan_migration(state: AppState, chip: String, into: RwSignal<Option<Migration>>) {
    #[derive(serde::Serialize)]
    struct Args {
        chip: String,
    }
    let args = Args { chip };
    track(
        state,
        async move { ipc::call::<_, Migration>(cmd::migrate::PLAN, &args).await },
        move |plan| into.set(Some(plan)),
    );
}

/// Carry one out, then re-read the project: the chip, the target and the
/// toolchain the status bar shows have all just changed.
pub fn apply_migration(state: AppState, plan: Migration, into: RwSignal<Option<Migration>>) {
    #[derive(serde::Serialize)]
    struct Args {
        plan: Migration,
    }
    let args = Args { plan };
    track(
        state,
        async move { ipc::call::<_, Vec<String>>(cmd::migrate::APPLY, &args).await },
        move |written| {
            into.set(None);
            for path in written {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: format!("— switched {path}"),
                    level: None,
                });
            }
            refresh_project(state);
        },
    );
}
