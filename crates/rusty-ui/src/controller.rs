//! The only place a cross-layer action begins.
//!
//! Views render; controllers fetch, mutate, and record failure. Keeping that
//! split means the busy indicator, the error surface, and the ordering rules
//! live in one file instead of being re-invented per panel.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_core::{FeatureImpact, FeatureRow, FeatureSelection, WorkspaceReport};
use rusty_embed::{
    Board, Chip, CommandPlan, EmbeddedProject, Explanation, Firmware, FlashAction, LogLevel,
    LogLine, LogStream, MemoryReport, Migration, Probe, RelocateReport, SerialPort,
    StorageLocation, ToolchainReport, Transport, WizardChoice, WizardOption,
};

use rusty_ai::{AgentEvent, ChatEvent, Message, Preset, ProviderConfig, ToolDef};
use rusty_edit::{Document, Entry, Line as EditLine};
use rusty_lsp::{HoverInfo, LspEvent};
use rusty_term::Screen as TermScreen;

use crate::{
    ipc::{self, Answer, cmd},
    state::{
        AppState, EditHistory, LspStatus, ParkedEditor, ToolRun, TraceClock, carried_provider,
    },
};

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
    state.in_flight.update(|n| *n += 1);
    spawn_local(async move {
        match future.await {
            Ok(value) => {
                state.error.set(None);
                apply(value);
            }
            Err(e) => {
                if owns_session {
                    // A session that failed to start is a session that is not
                    // running — otherwise a tool that is not installed leaves
                    // the Stop button up and the prompt refusing to send.
                    state.session_running.set(false);
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
                state.error.set(Some(e));
            }
        }
        state.in_flight.update(|n| *n = n.saturating_sub(1));
    });
}

/// Load the static catalogue. Cheap, and needed before the wizard or the
/// device list can render anything.
pub fn load_catalog(state: AppState) {
    track(
        state,
        ipc::get::<Vec<Chip>>(cmd::catalog::CHIPS),
        move |chips| state.chips.set(chips),
    );
    track(
        state,
        ipc::get::<Vec<Board>>(cmd::catalog::BOARDS),
        move |boards| state.boards.set(boards),
    );
}

pub fn load_recents(state: AppState) {
    spawn_local(async move {
        if let Ok(list) = ipc::get::<Vec<String>>(cmd::workbench::RECENTS).await {
            state.recents.set(list);
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
                    state.error.set(Some(error));
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
            Err(e) => state.error.set(Some(e)),
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
            state.project.set(Some(result.project));
            state.workspace.set(result.workspace);
            if let Some(detail) = result.workspace_error {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("cargo metadata is unavailable: {detail}"),
                    level: Some(LogLevel::Warn),
                });
            }
            // Another project's binary is worse than none — it would be analysed
            // against this project's chip and report plausible nonsense.
            state.selected_firmware.set(None);
            state.memory.set(None);
            state.document.set(None);
            state.draft.set(String::new());
            state.tabs.set(Vec::new());
            state.parked.set(Vec::new());
            state.history.set(EditHistory::default());
            state.completion.set(None);
            state.signature.set(None);
            state.search_query.set(String::new());
            state.search_results.set(None);
            state.search_word.set(false);
            state.find_open.set(false);
            state.find_replace_open.set(false);
            state.find_query.set(String::new());
            state.find_index.set(0);
            state.search_regex.set(false);
            state.search_include.set(String::new());
            state.search_exclude.set(String::new());
            state.file_tree.set(Vec::new());
            state.expanded.set(Vec::new());
            state.highlighted.set(Vec::new());
            state.echo_text.set(String::new());
            state.diagnostics.set(std::collections::HashMap::new());
            state.hover.set(None);
            state.reveal.set(None);
            // The selection names a member of the *previous* workspace, so
            // keeping it would ask the backend to resolve features for a package
            // that is not there.
            state.feature_selection.set(None);
            state.feature_rows.set(Vec::new());
            state.feature_impact.set(None);
            // The toolchain verdict depends on which chip was just detected, so
            // it has to follow rather than run alongside.
            refresh_toolchain(state);
            refresh_firmware(state);
            refresh_tree(state);
            start_lsp(state);
            if let Some(root) = state
                .project
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
            state.project.set(Some(project));
            refresh_toolchain(state);
            refresh_firmware(state);
            refresh_workspace(state);
            refresh_tree(state);
            start_lsp(state);
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
            Ok(report) => state.workspace.set(Some(report)),
            Err(e) => {
                state.workspace.set(None);
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
            state.selected_firmware.update(|selected| {
                if let Some(path) = selected.as_deref()
                    && !found.iter().any(|f| f.path == path)
                {
                    *selected = None;
                }
            });
            state.firmware.set(found);
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
        move |report| state.memory.set(Some(report)),
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

    state.feature_selection.set(Some(selection.clone()));

    let rows_args = Args {
        selection: selection.clone(),
    };
    track(
        state,
        async move { ipc::call::<_, Vec<FeatureRow>>(cmd::features::ROWS, &rows_args).await },
        move |rows| state.feature_rows.set(rows),
    );

    let impact_args = Args { selection };
    track(
        state,
        async move { ipc::call::<_, FeatureImpact>(cmd::features::IMPACT, &impact_args).await },
        move |impact| state.feature_impact.set(Some(impact)),
    );
}

pub fn refresh_toolchain(state: AppState) {
    track(
        state,
        ipc::get::<ToolchainReport>(cmd::toolchain::REPORT),
        move |report| state.toolchain.set(Some(report)),
    );
}

/// Reattach to a project the backend still holds, after a frontend reload.
pub fn restore(state: AppState) {
    if !ipc::backend_available() {
        state.error.set(Some(ipc::IpcError {
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

/// Ask what could complete at the caret.
///
/// The buffer is synced to the server first, without waiting for the pulse:
/// completion after typing `.` is about the text as of *that keystroke*, and a
/// 250ms-stale server answers about the wrong world. `did_change` dedups, so
/// the extra sync costs nothing when the pulse already ran.
pub fn request_completion(state: AppState, path: String, line: u32, col: u32, word_start: u32) {
    #[derive(serde::Serialize)]
    struct Sync {
        path: String,
        text: String,
    }
    #[derive(serde::Serialize)]
    struct Ask {
        path: String,
        line: u32,
        col: u32,
    }

    if state.lsp_status.get_untracked() != LspStatus::Ready {
        return;
    }
    let sync = Sync {
        path: path.clone(),
        text: state.draft.get_untracked(),
    };
    let ask = Ask {
        path: path.clone(),
        line,
        col,
    };
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::lsp::CHANGE, &sync).await;
        if let Ok(items) =
            ipc::call::<_, Vec<rusty_lsp::CompletionItem>>(cmd::lsp::COMPLETE, &ask).await
        {
            let current = state
                .document
                .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
            if current.as_deref() == Some(path.as_str()) && !items.is_empty() {
                state.completion.set(Some(crate::state::CompletionPopup {
                    path,
                    line,
                    word_start,
                    items,
                }));
            }
        }
    });
}

/// Ask what call the caret sits inside, for the signature card.
///
/// Syncs the draft first, like completion does: an answer about stale text
/// highlights the wrong parameter.
pub fn request_signature(state: AppState, path: String, line: u32, col: u32) {
    #[derive(serde::Serialize)]
    struct Sync {
        path: String,
        text: String,
    }
    #[derive(serde::Serialize)]
    struct Ask {
        path: String,
        line: u32,
        col: u32,
    }

    if state.lsp_status.get_untracked() != LspStatus::Ready {
        return;
    }
    let sync = Sync {
        path: path.clone(),
        text: state.draft.get_untracked(),
    };
    let ask = Ask {
        path: path.clone(),
        line,
        col,
    };
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::lsp::CHANGE, &sync).await;
        let answer = ipc::call::<_, Option<rusty_lsp::SignatureInfo>>(cmd::lsp::SIGNATURE, &ask)
            .await
            .ok()
            .flatten();
        let current = state
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
        if current.as_deref() == Some(path.as_str()) {
            // None clears: the server saying "no call here" is how the card
            // learns the caret left the parentheses.
            state.signature.set(answer.map(|info| (path, line, info)));
        }
    });
}

/// Ask what quick fixes exist at the caret, after syncing the draft — an
/// answer about stale text splices into the wrong place.
pub fn request_actions(state: AppState, path: String, line: u32, col: u32) {
    #[derive(serde::Serialize)]
    struct Sync {
        path: String,
        text: String,
    }
    #[derive(serde::Serialize)]
    struct Ask {
        path: String,
        line: u32,
        col: u32,
    }

    if state.lsp_status.get_untracked() != LspStatus::Ready {
        return;
    }
    let sync = Sync {
        path: path.clone(),
        text: state.draft.get_untracked(),
    };
    let ask = Ask {
        path: path.clone(),
        line,
        col,
    };
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::lsp::CHANGE, &sync).await;
        let Ok(fixes) =
            ipc::call::<_, Vec<rusty_lsp::CodeActionFix>>(cmd::lsp::ACTIONS, &ask).await
        else {
            return;
        };
        let current = state
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
        if current.as_deref() == Some(path.as_str()) {
            if fixes.is_empty() {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: "no quick fixes at the cursor".to_string(),
                    level: None,
                });
            } else {
                state.actions.set(Some((path, line, fixes)));
            }
        }
    });
}

/// Ask for the document's semantic colouring, and keep it only if the answer
/// still describes what is on screen.
pub fn request_semantic(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    if !path.ends_with(".rs") || state.lsp_status.get_untracked() != LspStatus::Ready {
        return;
    }
    let args = Args { path: path.clone() };
    spawn_local(async move {
        // Errors and empties are the warm-up talking; the lexical base colour
        // stays up either way, so there is nothing to report.
        let Ok(spans) =
            ipc::call::<_, Vec<rusty_lsp::SemanticSpan>>(cmd::lsp::SEMANTIC, &args).await
        else {
            return;
        };
        let current = state
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
        if current.as_deref() == Some(path.as_str()) && !spans.is_empty() {
            state.semantic.set(Some((path, spans)));
        }
    });
}

// ─── network ─────────────────────────────────────────────────────────────────

/// What proxy is stored, and what detection currently sees.
pub fn load_proxy_setting(stored: RwSignal<Option<String>>, detected: RwSignal<Option<String>>) {
    spawn_local(async move {
        if let Ok(value) = ipc::get::<serde_json::Value>(cmd::workbench::PROXY).await {
            stored.set(value["stored"].as_str().map(str::to_string));
            detected.set(value["detected"].as_str().map(str::to_string));
        }
    });
}

/// Store the proxy choice and re-read, so the preview line tells the truth.
pub fn save_proxy_setting(
    value: Option<String>,
    stored: RwSignal<Option<String>>,
    detected: RwSignal<Option<String>>,
    saved: RwSignal<bool>,
) {
    #[derive(serde::Serialize)]
    struct Args {
        value: Option<String>,
    }
    let args = Args { value };
    spawn_local(async move {
        if ipc::call::<_, ()>(cmd::workbench::SET_PROXY, &args)
            .await
            .is_ok()
        {
            saved.set(true);
            load_proxy_setting(stored, detected);
        }
    });
}

/// Board files that would not parse. Asked for by the Catalogue settings —
/// a user whose board never appears deserves the reason in the window, not
/// only in the CLI.
pub fn load_catalog_problems(state: AppState) {
    track(
        state,
        ipc::get::<Vec<rusty_embed::CatalogProblem>>(cmd::catalog::PROBLEMS),
        move |problems| state.catalog_problems.set(problems),
    );
}

/// Whether a key for this profile is on file. The key itself is never read
/// back — the credential store is write-only from here, by design.
pub fn refresh_key_state(state: AppState, profile: String) {
    #[derive(serde::Serialize)]
    struct Args {
        profile: String,
    }
    let args = Args { profile };
    spawn_local(async move {
        if let Ok(stored) = ipc::call::<_, bool>(cmd::ai::KEY_CONFIGURED, &args).await {
            state.ai_key_stored.set(stored);
        }
    });
}

/// Forget the stored key for a profile.
pub fn delete_key(state: AppState, profile: String) {
    #[derive(serde::Serialize)]
    struct Args {
        profile: String,
    }
    let args = Args {
        profile: profile.clone(),
    };
    spawn_local(async move {
        match ipc::call::<_, ()>(cmd::ai::DELETE_KEY, &args).await {
            Ok(()) => {
                state.ai_key_stored.set(false);
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: format!("the key for {profile} was removed"),
                    level: None,
                });
            }
            Err(error) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: error.message,
                level: Some(LogLevel::Error),
            }),
        }
    });
}

/// Forget what the last board said.
///
/// Every capture the protocol feeds, dropped together at the start of a run.
/// Partially clearing it is worse than not clearing at all: a plot whose
/// curves come from two different boots, on two different clocks, is read as
/// one signal and is not one.
fn clear_capture(state: AppState) {
    state.sim_gpio.set(std::collections::HashMap::new());
    state.sim_trace.set(crate::state::SimTrace::default());
    state.sim_display.set(String::new());
    state.plot.set(crate::state::Plot::default());
    state.params.set(Vec::new());
}

/// One line from a board, wherever it came from.
///
/// The same firmware prints the same protocol whether it is running in QEMU,
/// under `espflash monitor`, or on a port rusty holds open itself — so the
/// reading of it lives in one place rather than being re-implemented per
/// stream. It was per-stream once, and the consequence was that telemetry
/// plotted in the simulator and vanished on real hardware.
///
/// Anything that is not protocol is log, unchanged.
fn absorb(state: AppState, line: LogLine) {
    if let Some(sample) = rusty_embed::protocol::parse_telemetry(&line.text) {
        record_plot(state, sample);
    } else if let Some(param) = rusty_embed::protocol::parse_param(&line.text) {
        // Newest wins, by name: the firmware re-announces after a change, and
        // what it says it took is the truth — a clamp is information, not a
        // mistake to hide.
        state.params.update(|params| {
            match params.iter_mut().find(|known| known.name == param.name) {
                Some(known) => *known = param,
                None => params.push(param),
            }
        });
    } else if let Some(report) = rusty_embed::parse_gpio_report(&line.text) {
        state.sim_gpio.update(|gpio| {
            for (pin, level) in &report.pins {
                gpio.insert(*pin, *level);
            }
        });
        record_trace(state, report);
    } else if let Some(text) = rusty_embed::parse_display_report(&line.text) {
        state.sim_display.set(text);
    } else {
        state.push_log(line);
    }
}

/// The host's own clock in microseconds, for streams the firmware left
/// unstamped. Shared by the pin trace and the plot so the two axes mean the
/// same thing when both fall back to it.
fn host_now_us() -> u64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| (p.now() * 1000.0) as u64)
        .unwrap_or(0)
}

/// One gpio report into the waveform capture.
///
/// The first report decides the clock: stamped reports run on the firmware's
/// systimer, unstamped ones on the host's arrival time — never both, because
/// a trace that mixes time bases is a trace that lies. Capped so an hour of
/// simulation cannot eat the tab.
fn record_trace(state: AppState, report: rusty_embed::GpioReport) {
    const CAP: usize = 200_000;

    state.sim_trace.update(|trace| {
        let clock = *trace.clock.get_or_insert(if report.at_us.is_some() {
            TraceClock::Firmware
        } else {
            TraceClock::Host
        });
        let at = match (clock, report.at_us) {
            (TraceClock::Firmware, Some(us)) => us,
            // A late unstamped line in a firmware-clocked trace (or the
            // reverse) would corrupt the axis; reuse the last moment
            // instead, which keeps the event without inventing a time.
            (TraceClock::Firmware, None) => trace.events.last().map(|(t, ..)| *t).unwrap_or(0),
            (TraceClock::Host, _) => host_now_us(),
        };
        for (pin, level) in report.pins {
            trace.events.push((at, pin, level));
        }
        if trace.events.len() > CAP {
            let drop = trace.events.len() - CAP;
            trace.events.drain(..drop);
            trace.truncated = true;
        }
    });
}

/// Fold one telemetry line into the rolling plot.
///
/// The same clock discipline the pin trace learned: whichever base the first
/// sample arrived on decides the axis, and a later line in the other base
/// reuses the last moment rather than inventing one. Mixing them silently is
/// how a plot lies about *when*, which for a control loop is the only thing
/// it is being read for.
fn record_plot(state: AppState, sample: rusty_embed::protocol::Telemetry) {
    /// Samples kept per channel. A 1 kHz loop fills this in twenty seconds,
    /// which is the window a tuning change is judged in; older than that and
    /// you re-run rather than scroll back.
    const CAP: usize = 20_000;

    state.plot.update(|plot| {
        let clock = *plot.clock.get_or_insert(if sample.at_us.is_some() {
            TraceClock::Firmware
        } else {
            TraceClock::Host
        });
        let at = match (clock, sample.at_us) {
            (TraceClock::Firmware, Some(us)) => us,
            (TraceClock::Firmware, None) => plot
                .channels
                .iter()
                .filter_map(|(_, points)| points.last().map(|(t, _)| *t))
                .max()
                .unwrap_or(0),
            (TraceClock::Host, _) => host_now_us(),
        };
        let mut dropped = false;
        for (name, value) in sample.channels {
            let points = plot.channel(&name);
            points.push((at, value));
            if points.len() > CAP {
                let over = points.len() - CAP;
                points.drain(..over);
                dropped = true;
            }
        }
        plot.truncated |= dropped;
    });
}

/// Set a tunable on the running firmware, over the serial line it is already
/// talking on.
///
/// No reflash: the whole point. The firmware answers with a `[rusty:param]`
/// line carrying what it actually took, and that is what the panel then
/// shows — so a value the firmware clamped reads as clamped rather than as
/// the number that was typed.
pub fn set_param(state: AppState, name: String, value: f32) {
    sim_send(state, rusty_embed::protocol::set_param_line(&name, value));
}

/// Hold a serial port open in both directions, until it is closed.
///
/// The mode a control loop is tuned in. `espflash monitor` reads its keyboard
/// through the console rather than through stdin, so a monitor rusty spawned
/// can only listen — this opens the port itself, which is what makes a
/// tunable writable, and gives up defmt decoding to do it.
pub fn open_link(state: AppState, port: String, baud: u32) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Serialize)]
    struct Args {
        port: String,
        baud: u32,
    }

    if state.session_running.get_untracked() {
        return;
    }
    state.log_source.set("link");
    clear_capture(state);

    let channel = ipc::Channel::new();
    let on_line =
        Closure::wrap(Box::new(move |value: JsValue| {
            match serde_wasm_bindgen::from_value::<LogLine>(value) {
                Ok(line) => absorb(state, line),
                Err(e) => state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("[rusty could not decode a line from the port: {e}]"),
                    level: Some(LogLevel::Warn),
                }),
            }
        }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_line);
    on_line.forget();

    state.session_running.set(true);
    state.link_port.set(Some(port.clone()));
    state.show_dock(crate::state::DockTab::Plot);

    let args = Args { port, baud };
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, Option<i32>>(cmd::flash::LINK, &args, "onLine", &channel).await
        },
        move |_| {
            state.link_port.set(None);
            note_exit(state, None);
        },
    );
}

/// Close the link, which is the same stop every other session answers to.
pub fn close_link(state: AppState) {
    state.link_port.set(None);
    stop_session(state);
}

/// Write the captured trace as VCD into the project's target directory,
/// where PulseView and GTKWave can open it.
pub fn export_vcd(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        text: String,
    }

    let trace = state.sim_trace.get_untracked();
    if trace.events.is_empty() {
        return;
    }
    let args = Args {
        text: rusty_embed::to_vcd(&trace.events),
    };
    spawn_local(async move {
        match ipc::call::<_, String>(cmd::sim::SAVE_TRACE, &args).await {
            Ok(path) => {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: format!("waveform written to {path} — PulseView and GTKWave open it"),
                    level: None,
                });
                state.show_dock(crate::state::DockTab::Output);
            }
            Err(error) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: error.message,
                level: Some(LogLevel::Error),
            }),
        }
    });
}

/// `cargo build --release` for the open project, streamed to the dock —
/// the coding toolbar's Build, sharing the one session slot with
/// everything else that runs.
pub fn build_project(state: AppState) {
    if state.session_running.get_untracked() {
        return;
    }
    run_session(
        state,
        CommandPlan {
            program: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
            display: "cargo build --release".to_string(),
            rationale: "the project's own toolchain builds the exact firmware a device \
                        would get"
                .to_string(),
            warning: None,
        },
        "build",
    );
}

// ─── crates ──────────────────────────────────────────────────────────────────

/// Ask crates.io about every direct dependency. Slow by design — one index
/// request per crate — so only the panel's own ask triggers it.
pub fn load_crate_report(state: AppState) {
    if !state.has_project() {
        return;
    }
    state.crate_rows.set(None);
    track(
        state,
        ipc::get::<Vec<rusty_core::CrateRow>>(cmd::crates::REPORT),
        move |rows| state.crate_rows.set(Some(rows)),
    );
}

/// `cargo add name@version` through the shared session slot, then re-analyse
/// — the manifest changed, so the old graph and the old rows are both stale.
pub fn upgrade_crate(state: AppState, name: String, version: String) {
    state.log_source.set("tools");
    if state.session_running.get_untracked() || version.is_empty() {
        return;
    }
    let plan = CommandPlan {
        program: "cargo".to_string(),
        args: vec!["add".to_string(), format!("{name}@{version}")],
        display: format!("cargo add {name}@{version}"),
        rationale: "updates Cargo.toml to the requested version and re-resolves the lockfile"
            .to_string(),
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

// ─── simulation ──────────────────────────────────────────────────────────────

/// Ask how this project would be simulated.
pub fn load_sim_plan(state: AppState) {
    if !state.has_project() {
        return;
    }
    track(
        state,
        ipc::get::<rusty_embed::SimPlan>(cmd::sim::PLAN),
        move |plan| state.sim_plan.set(Some(plan)),
    );
}

/// Build, image and boot in QEMU, streaming into the dock. One at a time —
/// the shared session slot enforces it the same way flashing does.
pub fn run_simulation(state: AppState, debug: bool) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Serialize)]
    struct Args {
        debug: bool,
    }

    if state.session_running.get_untracked() {
        return;
    }
    state.log_source.set("simulate");
    clear_capture(state);

    // Like stream_to_terminal, with one interception: the firmware's pin
    // reports drive the board view instead of scrolling the dock at 2Hz.
    let channel = ipc::Channel::new();
    let on_line = Closure::wrap(Box::new(move |value: JsValue| {
        match serde_wasm_bindgen::from_value::<LogLine>(value) {
            Ok(line) => {
                // The debug sentinel: QEMU is frozen and listening. Open the
                // terminal and type the attach line for the user — the gdb
                // REPL is theirs from there.
                if line.text.starts_with("[rusty:debug]") {
                    state.push_log(line);
                    // QEMU is frozen with its gdbstub listening. Attach the
                    // in-app debugger: breakpoints in the gutter, the stack
                    // in the dock. gdb's own REPL is still a terminal away
                    // for anything the panel does not model. The run that
                    // armed it knows which ELF it booted and on which port —
                    // reading that from the panel's cached plan attached gdb
                    // to the release binary while the unoptimised one ran.
                    debug_start(state, false);
                    return;
                }
                absorb(state, line);
            }
            Err(e) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: format!("[rusty could not decode a line from the tool: {e}]"),
                level: Some(LogLevel::Warn),
            }),
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_line);
    on_line.forget();
    state.session_running.set(true);
    state.show_dock(crate::state::DockTab::Output);
    spawn_local(async move {
        match ipc::call_streaming::<_, Option<i32>>(
            cmd::sim::RUN,
            &Args { debug },
            "onLine",
            &channel,
        )
        .await
        {
            Ok(code) => note_exit(state, code),
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

/// One-click install of a missing simulator tool, streamed to the dock.
/// Success refreshes the plan; failure reveals the manual instructions.
pub fn install_sim_tool(state: AppState, name: String) {
    state.log_source.set("tools");
    #[derive(serde::Serialize)]
    struct Args {
        name: String,
    }

    if state.session_running.get_untracked() {
        return;
    }
    let args = Args { name: name.clone() };
    let channel = stream_to_terminal(state);
    spawn_local(async move {
        let outcome =
            ipc::call_streaming::<_, Option<i32>>(cmd::sim::INSTALL, &args, "onLine", &channel)
                .await;
        match outcome {
            Ok(Some(0)) => {
                note_exit(state, Some(0));
                state
                    .sim_install_failed
                    .update(|failed| failed.retain(|t| t != &name));
                // The world just changed: re-probe the machine, and give the
                // editor its language server the moment it exists.
                refresh_toolchain(state);
                if name == "rust-analyzer" {
                    start_lsp(state);
                }
            }
            Ok(code) => {
                note_exit(state, code);
                state.sim_install_failed.update(|failed| {
                    if !failed.contains(&name) {
                        failed.push(name.clone());
                    }
                });
            }
            Err(error) => {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: error.message,
                    level: Some(LogLevel::Error),
                });
                note_exit(state, Some(-1));
                state.sim_install_failed.update(|failed| {
                    if !failed.contains(&name) {
                        failed.push(name.clone());
                    }
                });
            }
        }
        // Either way the plan is re-asked: a success clears the card, and
        // even a failure may have changed the world (a partial unpack).
        load_sim_plan(state);
    });
}

/// One line into the running firmware's UART.
///
/// Fire-and-forget: a press against a stopped board lands nowhere, which is
/// what pressing a powered-off board does. Every input the panels have —
/// buttons, pots, tunables — is one of these, so the running check lives here
/// rather than being re-remembered at each call site.
fn sim_send(state: AppState, text: String) {
    #[derive(serde::Serialize)]
    struct Args {
        text: String,
    }
    if !state.session_running.get_untracked() {
        return;
    }
    let args = Args { text };
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::sim::SEND, &args).await;
    });
}

/// A button transition on the board view.
pub fn sim_press(state: AppState, pin: u8, down: bool) {
    sim_send(state, format!("B{pin}={}", if down { 1 } else { 0 }));
}

/// A potentiometer moved: `P<pin>=<0..255>` into the firmware's UART.
pub fn sim_pot(state: AppState, pin: u8, value: u8) {
    sim_send(state, format!("P{pin}={value}"));
}

/// Persist the board editor's layout, then re-plan so the panel shows what
/// the file now says.
pub fn save_sim_board(state: AppState, board: rusty_embed::SimBoard, dirty: RwSignal<bool>) {
    #[derive(serde::Serialize)]
    struct Args {
        board: rusty_embed::SimBoard,
    }
    let args = Args { board };
    spawn_local(async move {
        match ipc::call::<_, ()>(cmd::sim::SAVE_BOARD, &args).await {
            Ok(()) => {
                dirty.set(false);
                load_sim_plan(state);
            }
            Err(error) => {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("could not save the board: {}", error.message),
                    level: Some(LogLevel::Error),
                });
            }
        }
    });
}

/// Open the dock terminal and type the gdb attach line into it.
///
/// The shell does the launching, so the user sees exactly what ran and owns
/// the REPL afterwards — break, step, print are theirs, not wrapped.
/// Open gdb's own REPL in the terminal — the escape hatch for anything the
/// debug panel does not model (raw registers, `x/16x`, a scripted `commands`
/// block). The in-app session owns the ordinary path.
pub fn attach_debugger_terminal(state: AppState, command: String) {
    state.show_dock(crate::state::DockTab::Terminal);
    // `terminal` holds the shell's latest frame; None means no shell yet.
    if state.terminal.with_untracked(Option::is_none) {
        open_terminal(state, 100, 24);
    }
    #[derive(serde::Serialize)]
    struct Args {
        bytes: Vec<u8>,
    }
    // A freshly opened terminal needs a beat before the shell reads keys.
    set_timeout(
        move || {
            let args = Args {
                bytes: format!("{command}\r").into_bytes(),
            };
            spawn_local(async move {
                let _ = ipc::call::<_, ()>(cmd::terminal::WRITE, &args).await;
            });
        },
        std::time::Duration::from_millis(700),
    );
}

/// The panel-facing spelling of "stop whatever session is running".
pub fn stop_session_now(state: AppState) {
    stop_session(state);
}

// ─── session restore ─────────────────────────────────────────────────────────

/// localStorage key for a project's open tabs. Per the storage doctrine this
/// is WebView-only state whose loss costs a shrug — exactly localStorage's
/// province.
fn tabs_key(root: &str) -> String {
    format!("rusty.tabs.{root}")
}

/// Write the strip to localStorage: open paths, active one first.
pub fn remember_tabs(state: AppState) {
    // Nor to overwrite: one detached window remembering its single tab
    // would wipe the shell's strip for the next launch.
    if state.detached.with_untracked(Option::is_some) {
        return;
    }
    let Some(root) = state
        .project
        .with_untracked(|p| p.as_ref().map(|p| p.root.clone()))
    else {
        return;
    };
    let active = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
    let tabs = state.tabs.get_untracked();
    #[derive(serde::Serialize)]
    struct Args {
        root: String,
        tabs: Vec<String>,
        active: Option<String>,
    }
    let args = Args { root, tabs, active };
    // Fire and forget: a tab strip that failed to save is not worth a banner
    // over the edit the user was making when it happened.
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::workbench::RECORD_TABS, &args).await;
    });
}

/// Reopen the tabs the project had last time. Missing files fail their open
/// quietly through the normal error path; the strip simply ends up shorter.
pub fn restore_tabs(state: AppState, root: &str) {
    // A detached window edits one file; the shell's saved strip is not its
    // business to reopen.
    if state.detached.with_untracked(Option::is_some) {
        return;
    }
    // Anything this window still has from before the strip became a file.
    // Read once, handed over, and deleted — so an upgrade does not cost
    // somebody the tabs they had open, and the key does not linger to be
    // read again by a later version that no longer understands it.
    let carried = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| {
            let key = tabs_key(root);
            let raw = storage.get_item(&key).ok().flatten()?;
            let _ = storage.remove_item(&key);
            serde_json::from_str::<serde_json::Value>(&raw).ok()
        });

    #[derive(serde::Serialize)]
    struct Args {
        root: String,
    }
    let args = Args {
        root: root.to_string(),
    };
    spawn_local(async move {
        let stored =
            ipc::call::<_, Option<rusty_embed::ProjectTabs>>(cmd::workbench::PROJECT_TABS, &args)
                .await
                .ok()
                .flatten();

        let (tabs, active) = match (stored, carried) {
            // The file wins where it has an answer: it is the one that
            // survives a reinstall, and a stale WebView copy would undo the
            // last session every time.
            (Some(stored), _) => (stored.tabs, stored.active),
            (None, Some(old)) => (
                old["tabs"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|v| v.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                old["active"].as_str().map(str::to_string),
            ),
            (None, None) => return,
        };

        // Open in strip order; the active one last, so it ends up on screen.
        for path in tabs
            .iter()
            .filter(|p| Some(p.as_str()) != active.as_deref())
        {
            open_file(state, path.clone());
        }
        if let Some(active) = active {
            open_file(state, active);
        }
    });
}

// ─── project search ─────────────────────────────────────────────────────────────

/// Debounced: called on every keystroke in the search box, runs the search
/// only when the typing pauses.
pub fn schedule_search(state: AppState) {
    let generation = state.search_gen.get_untracked() + 1;
    state.search_gen.set(generation);
    set_timeout(
        move || {
            if state.search_gen.get_untracked() == generation {
                run_search(state, generation);
            }
        },
        std::time::Duration::from_millis(250),
    );
}

fn run_search(state: AppState, generation: u64) {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        query: String,
        case_sensitive: bool,
        whole_word: bool,
        regex: bool,
        include: String,
        exclude: String,
    }

    let query = state.search_query.get_untracked();
    if query.trim().is_empty() {
        state.search_results.set(None);
        return;
    }
    let args = Args {
        query,
        case_sensitive: state.search_case.get_untracked(),
        whole_word: state.search_word.get_untracked(),
        regex: state.search_regex.get_untracked(),
        include: state.search_include.get_untracked(),
        exclude: state.search_exclude.get_untracked(),
    };
    spawn_local(async move {
        let result = ipc::call::<_, rusty_edit::SearchResults>(cmd::files::SEARCH, &args).await;
        // Typed since? This answer is about a query nobody is asking anymore.
        if state.search_gen.get_untracked() != generation {
            return;
        }
        match result {
            Ok(results) => state.search_results.set(Some(results)),
            Err(_) => state.search_results.set(None),
        }
    });
}

/// Open a file at an exact position — how a search hit or a problem row
/// lands in the editor, through the same reveal goto-definition uses.
pub fn open_at(state: AppState, path: String, line: u32, col: u32) {
    // Files and Search both keep an editor on the right, VSCode-style; a
    // jump from either stays put. From anywhere else, land in Files.
    let panel = state.active_panel.get_untracked();
    if panel != "files" && panel != "search" {
        state.active_panel.set("files".to_string());
    }
    let current = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
    if current.as_deref() != Some(path.as_str()) {
        open_file(state, path.clone());
    }
    state.reveal.set(Some(rusty_lsp::Location {
        path,
        line,
        col,
        external: false,
    }));
}

// ─── storage ─────────────────────────────────────────────────────────────────

/// Where the data directory is, for the settings screen.
pub fn load_storage_footprint(into: RwSignal<Option<u64>>) {
    spawn_local(async move {
        if let Ok(bytes) = ipc::get::<u64>(cmd::workbench::FOOTPRINT).await {
            into.set(Some(bytes));
        }
    });
}

pub fn load_storage_location(into: RwSignal<Option<StorageLocation>>) {
    spawn_local(async move {
        if let Ok(found) =
            ipc::get::<Option<StorageLocation>>(cmd::workbench::STORAGE_LOCATION).await
        {
            into.set(found);
        }
    });
}

/// Ask for a folder, then move the data directory into it.
///
/// The refused-because-occupied case is separated from other failures so the
/// screen can offer "adopt what is there" as a deliberate second step rather
/// than a checkbox nobody reads the first time.
pub fn relocate_storage(
    state: AppState,
    target: String,
    take_existing: bool,
    note: RwSignal<Option<String>>,
    blocked: RwSignal<Option<String>>,
    location: RwSignal<Option<StorageLocation>>,
) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        take_existing: bool,
    }

    let args = Args {
        path: target.clone(),
        take_existing,
    };
    spawn_local(async move {
        match ipc::call::<_, RelocateReport>(cmd::workbench::RELOCATE, &args).await {
            Ok(report) => {
                blocked.set(None);
                note.set(Some(if report.adopted {
                    format!("Now using the data already in {}.", report.to)
                } else {
                    format!(
                        "Moved: {} files copied to {}. The originals are still in {} —                          delete them yourself once you are satisfied.",
                        report.copied_files, report.to, report.from,
                    )
                }));
                load_storage_location(location);
                // The recents list travelled with the directory.
                load_recents(state);
                load_catalog(state);
            }
            Err(error) => {
                if error.message.contains("already contains rusty data") {
                    blocked.set(Some(target.clone()));
                }
                note.set(Some(error.message));
            }
        }
    });
}

/// The folder picker, for the storage screen.
pub fn pick_storage_folder(on: Callback<Option<String>>) {
    spawn_local(async move {
        let picked = ipc::pick_folder("Where should rusty keep its data?")
            .await
            .ok()
            .flatten();
        on.run(picked);
    });
}

// ─── the assistant ───────────────────────────────────────────────────────────

/// Presets and the tool list. Both static, so once per session.
pub fn load_assistant(state: AppState) {
    track(
        state,
        ipc::get::<Vec<Preset>>(cmd::ai::PRESETS),
        move |presets| state.ai_presets.set(presets),
    );
    track(
        state,
        ipc::get::<Vec<ToolDef>>(cmd::ai::TOOLS),
        move |tools| state.ai_tools.set(tools),
    );
}

/// Save the provider profile. The key is handled separately and never comes back.
pub fn set_provider(state: AppState, config: ProviderConfig) {
    #[derive(serde::Serialize)]
    struct Args {
        choice: rusty_embed::AssistantChoice,
    }
    let args = Args {
        choice: to_choice(&config),
    };
    state.ai_config.set(Some(config));
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::workbench::SET_ASSISTANT, &args).await;
    });
}

/// The profile last chosen, from the file — falling back to whatever this
/// window still holds from before it was one, which is then written through
/// so the next launch reads it from the file like everything else.
pub fn load_provider(state: AppState) {
    spawn_local(async move {
        let stored =
            ipc::call::<_, Option<rusty_embed::AssistantChoice>>(cmd::workbench::ASSISTANT, &())
                .await
                .ok()
                .flatten();
        match stored.and_then(|choice| from_choice(&choice)) {
            Some(config) => state.ai_config.set(Some(config)),
            None => {
                if let Some(config) = carried_provider() {
                    set_provider(state, config);
                }
            }
        }
    });
}

/// The file's shape and the frontend's are separate types on purpose — the
/// same rule that keeps the chip catalogue's file format out of `model`.
fn to_choice(config: &ProviderConfig) -> rusty_embed::AssistantChoice {
    rusty_embed::AssistantChoice {
        profile: config.profile.clone(),
        kind: serde_json::to_value(config.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        base_url: config.base_url.clone(),
        model: config.model.clone(),
        max_tokens: Some(config.max_tokens),
        temperature: config.temperature,
        supports_tools: Some(config.supports_tools),
    }
}

fn from_choice(choice: &rusty_embed::AssistantChoice) -> Option<ProviderConfig> {
    Some(ProviderConfig {
        profile: choice.profile.clone(),
        kind: serde_json::from_value(serde_json::Value::String(choice.kind.clone())).ok()?,
        base_url: choice.base_url.clone(),
        model: choice.model.clone(),
        max_tokens: choice.max_tokens.unwrap_or(4096),
        temperature: choice.temperature,
        supports_tools: choice.supports_tools.unwrap_or(true),
    })
}

/// File an API key in the OS credential store.
pub fn store_key(state: AppState, profile: String, api_key: String) {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        profile: String,
        api_key: String,
    }

    let args = Args { profile, api_key };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::ai::STORE_KEY, &args).await },
        move |()| {},
    );
}

/// Ask the endpoint which models it serves.
///
/// Discovered rather than hardcoded: model names drift far faster than a
/// release cycle, and a self-hosted server's names are unknowable in advance.
pub fn list_models(state: AppState, config: ProviderConfig, into: RwSignal<Vec<String>>) {
    #[derive(serde::Serialize)]
    struct Args {
        config: ProviderConfig,
    }

    let args = Args { config };
    track(
        state,
        async move { ipc::call::<_, Vec<String>>(cmd::ai::LIST_MODELS, &args).await },
        move |models| into.set(models),
    );
}

/// Check a profile end to end without starting a conversation.
pub fn check_provider(state: AppState, config: ProviderConfig, into: RwSignal<Option<String>>) {
    #[derive(serde::Serialize)]
    struct Args {
        config: ProviderConfig,
    }

    let args = Args { config };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::ai::CHECK_PROVIDER, &args).await },
        move |verdict| into.set(Some(verdict)),
    );
}

/// Ask a question, streaming the answer.
pub fn ask(state: AppState, question: String) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    let Some(config) = state.ai_config.get_untracked() else {
        return;
    };

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        config: ProviderConfig,
        history: Vec<Message>,
    }

    state
        .conversation
        .update(|c| c.push(Message::user(question)));
    state.ai_pending.set(String::new());
    state.ai_activity.set(Vec::new());
    state.ai_usage.set(None);
    state.ai_streaming.set(true);

    let channel = ipc::Channel::new();
    let on_event = Closure::wrap(Box::new(move |value: JsValue| {
        if let Ok(event) = serde_wasm_bindgen::from_value::<AgentEvent>(value) {
            apply_event(state, event);
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_event);
    // Held by the backend for the length of the answer, which outlives this
    // call. One per question asked.
    on_event.forget();

    let args = Args {
        config,
        history: state.conversation.get_untracked(),
    };
    track(
        state,
        async move {
            ipc::call_streaming::<_, Vec<Message>>(cmd::ai::ASK, &args, "onEvent", &channel).await
        },
        move |history| {
            // The backend's history is authoritative — it contains the tool
            // calls and their results, which the stream only summarised. Keeping
            // the locally-accumulated text instead would send a transcript back
            // next turn that the model never actually produced.
            state.conversation.set(history);
            state.ai_pending.set(String::new());
            state.ai_streaming.set(false);
        },
    );
}

/// Fold one streamed event into the visible state.
fn apply_event(state: AppState, event: AgentEvent) {
    match event {
        AgentEvent::Chat(ChatEvent::TextDelta { text }) => {
            state.ai_pending.update(|pending| pending.push_str(&text));
        }
        AgentEvent::Chat(ChatEvent::Usage {
            input_tokens,
            output_tokens,
        }) => state.ai_usage.set(Some((input_tokens, output_tokens))),
        AgentEvent::ToolStarted { id, name, .. } => {
            state
                .ai_activity
                .update(|runs| runs.push(ToolRun { id, name, ok: None }));
        }
        AgentEvent::ToolFinished { id, ok, .. } => {
            state.ai_activity.update(|runs| {
                if let Some(run) = runs.iter_mut().find(|r| r.id == id) {
                    run.ok = Some(ok);
                }
            });
        }
        // The provider-level tool-call events restate what `ToolStarted` and
        // `ToolFinished` already say, but without the agent loop's knowledge of
        // whether the call succeeded. Rendering both would double every row.
        AgentEvent::Chat(_) => {}
    }
}

/// Throw away the transcript.
pub fn clear_conversation(state: AppState) {
    state.conversation.set(Vec::new());
    state.ai_pending.set(String::new());
    state.ai_activity.set(Vec::new());
    state.ai_usage.set(None);
}

// ─── new project ─────────────────────────────────────────────────────────────

/// Load the generator's options. Static, so once per session is enough.
pub fn load_wizard_options(state: AppState) {
    track(
        state,
        ipc::get::<Vec<WizardOption>>(cmd::wizard::OPTIONS),
        move |options| state.wizard_options.set(options),
    );
}

/// Record a choice and ask what it commits the user to.
///
/// Called on every change rather than at the end. A wizard that explains itself
/// only after the last step is a wizard that explains nothing — the point is to
/// answer "what does this mean" while the answer can still change the choice.
pub fn choose(state: AppState, choice: WizardChoice) {
    #[derive(serde::Serialize)]
    struct Args {
        choice: WizardChoice,
    }

    state.wizard_choice.set(Some(choice.clone()));

    let explain_args = Args {
        choice: choice.clone(),
    };
    track(
        state,
        async move { ipc::call::<_, Vec<Explanation>>(cmd::wizard::EXPLAIN, &explain_args).await },
        move |explanations| state.wizard_explanations.set(explanations),
    );

    // The plan can legitimately fail — a chip with no `std` target under the
    // ESP-IDF runtime, say — and that refusal is the useful answer. It surfaces
    // through the normal error path and clears the stale command.
    state.wizard_plan.set(None);
    let plan_args = Args { choice };
    track(
        state,
        async move { ipc::call::<_, CommandPlan>(cmd::wizard::PLAN, &plan_args).await },
        move |plan| state.wizard_plan.set(Some(plan)),
    );
}

/// Ask where it should go, generate it, then open it.
///
/// The command is still shown — that is what makes the tool inspectable — but
/// showing it *instead* of acting made this panel a slow way to type. The one
/// decision rusty must not make quietly is where the code lands, and that is
/// exactly what the folder picker asks.
pub fn create_project(state: AppState, choice: WizardChoice) {
    state.log_source.set("tools");
    #[derive(serde::Serialize)]
    struct Args {
        choice: WizardChoice,
        directory: String,
    }

    spawn_local(async move {
        // Cancelling is not a failure and must not surface as one.
        let directory = match ipc::pick_folder("Where should the project go?").await {
            Ok(Some(directory)) => directory,
            Ok(None) => return,
            Err(e) => {
                state.error.set(Some(e));
                return;
            }
        };

        let channel = stream_to_terminal(state);
        let args = Args { choice, directory };
        track_session(
            state,
            async move {
                ipc::call_streaming::<_, String>(cmd::wizard::CREATE, &args, "onLine", &channel)
                    .await
            },
            move |path| {
                state.session_running.set(false);
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: format!("— created {path}"),
                    level: Some(LogLevel::Info),
                });
                // Opening it is the whole point: a wizard that generates a
                // project and then leaves you looking at the wizard has stopped
                // one step short of being useful.
                open_project(state, path);
                // And go there. Staying on the review step leaves the screen
                // describing a decision that has already been carried out,
                // while the thing it produced is somewhere the user has to go
                // and find.
                state.active_panel.set("files".to_string());
            },
        );
    });
}

// ─── devices ─────────────────────────────────────────────────────────────────

/// Re-enumerate serial ports and debug probes.
///
/// Explicit rather than polled. Enumerating serial ports opens each device on
/// some platforms, and doing that on a timer while a monitor is attached is a
/// good way to disturb the session the user is watching.
pub fn scan_devices(state: AppState) {
    track(
        state,
        ipc::get::<Vec<SerialPort>>(cmd::flash::SERIAL_PORTS),
        move |ports| {
            // Keep a chosen port only while it is still attached. Silently
            // holding a disconnected one means the next flash fails with an
            // access error naming a device the user already unplugged.
            state.transport.update(|transport| {
                if let Some(Transport::Serial { port }) = transport.as_ref()
                    && !ports.iter().any(|p| &p.name == port)
                {
                    *transport = None;
                }
            });
            state.ports.set(ports);
        },
    );
    track(
        state,
        ipc::get::<Vec<Probe>>(cmd::flash::DEBUG_PROBES),
        move |probes| state.probes.set(probes),
    );
}

/// Work out the command for the current device, firmware and action.
///
/// Always run before flashing, and the result is shown verbatim. Embedded work
/// happens in a terminal as much as in a window; a button that hides what it
/// runs is a button people work around.
pub fn plan_session(state: AppState, action: FlashAction) {
    let (Some(transport), Some(firmware)) = (state.transport.get(), state.current_firmware())
    else {
        state.plan.set(None);
        return;
    };

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        transport: Transport,
        action: FlashAction,
        firmware: String,
        defmt: bool,
        baud: Option<u32>,
    }

    // Whether to decode defmt is a property of the binary, not a preference:
    // asking espflash to decode a build without the string table produces
    // gibberish, and not asking on a build with one produces framing bytes.
    let defmt = state
        .project
        .with(|p| p.as_ref().is_some_and(|p| p.uses_defmt));

    let args = Args {
        transport,
        action,
        firmware: firmware.path,
        defmt,
        baud: None,
    };
    track(
        state,
        async move { ipc::call::<_, CommandPlan>(cmd::flash::PLAN, &args).await },
        move |plan| state.plan.set(Some(plan)),
    );
}

/// A channel whose every line lands in the terminal, and the dock brought
/// forward to show it.
///
/// Shared by flashing, monitoring, project generation and the terminal itself:
/// four things that spawn a tool, and one place their output goes. Splitting
/// them into separate views would mean a failed flash and the build that caused
/// it appearing in different panes.
fn stream_to_terminal(state: AppState) -> ipc::Channel {
    use wasm_bindgen::{JsValue, prelude::Closure};

    let channel = ipc::Channel::new();
    let on_line = Closure::wrap(Box::new(move |value: JsValue| {
        match serde_wasm_bindgen::from_value::<LogLine>(value) {
            // Through `absorb` rather than straight to the log: a board on the
            // end of `espflash monitor` prints the same telemetry it prints in
            // the simulator, and the plot should draw it. Writing back is what
            // that mode cannot do, not reading.
            Ok(line) => absorb(state, line),
            // A line that will not decode is still worth showing: it means the
            // wire type and the tool disagree, and silently dropping output is
            // the one thing a monitor must never do.
            Err(e) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: format!("[rusty could not decode a line from the tool: {e}]"),
                level: Some(LogLevel::Warn),
            }),
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_line);
    // Deliberately leaked: the backend holds this channel for the life of the
    // session, which outlives this call. One closure per run, freed never —
    // bounded by how many times a person presses a button.
    on_line.forget();

    state.session_running.set(true);
    state.show_dock(crate::state::DockTab::Output);
    channel
}

/// Note how a spawned tool ended, in the terminal where its output is.
fn note_exit(state: AppState, code: Option<i32>) {
    state.session_running.set(false);
    let source = state.log_source;
    set_timeout(move || source.set("app"), std::time::Duration::ZERO);
    let text = match code {
        Some(0) | None => "— finished".to_string(),
        Some(code) => format!("— exited with status {code}"),
    };
    state.push_log(LogLine {
        stream: LogStream::Stdout,
        text,
        level: None,
    });
}

/// Run the planned command, streaming its output into the terminal.
pub fn run_session(state: AppState, plan: CommandPlan, channel: &'static str) {
    state.log_source.set(channel);
    #[derive(serde::Serialize)]
    struct Args {
        plan: CommandPlan,
    }

    let channel = stream_to_terminal(state);
    let args = Args { plan };
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, Option<i32>>(cmd::flash::RUN, &args, "onLine", &channel).await
        },
        move |code| note_exit(state, code),
    );
}

// ─── files ───────────────────────────────────────────────────────────────────

/// Create a file or directory, then show it — the tree refreshes, and a new
/// file opens in the editor, because "New file" means "I want to type in it".
pub fn create_entry(state: AppState, path: String, dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        dir: bool,
    }

    if !state.has_project() {
        return;
    }
    let opened = path.clone();
    let args = Args { path, dir };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::CREATE, &args).await },
        move |()| {
            refresh_tree(state);
            if !dir {
                open_file(state, opened.clone());
            }
        },
    );
}

/// Float a file into its own OS window.
pub fn detach_file(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::DETACH, &Args { path }).await },
        |()| {},
    );
}

/// Push the remembered interface scale to the webview. Through `track`, so
/// "command not found" — the stale-backend symptom — surfaces as a banner
/// instead of a slider that silently does nothing.
pub fn apply_ui_zoom(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        factor: f64,
    }
    let factor = state.ui_zoom.get_untracked();
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::window::SET_ZOOM, &Args { factor }).await },
        |()| {},
    );
}

/// What shell the terminal will start.
/// The shells the picker can offer. Loaded once; a machine does not grow
/// shells mid-session often enough to poll for.
pub fn load_shell_choices(state: AppState) {
    if state.shell_choices.with_untracked(|c| !c.is_empty()) {
        return;
    }
    track(
        state,
        async move { ipc::get::<Vec<rusty_embed::ShellChoice>>(cmd::terminal::SHELLS).await },
        move |choices| state.shell_choices.set(choices),
    );
}

pub fn load_shell_info(state: AppState) {
    track(
        state,
        async move { ipc::call::<_, rusty_embed::ShellInfo>(cmd::terminal::SHELL_INFO, &()).await },
        move |info| state.shell_info.set(Some(info)),
    );
}

/// Store the shell preference and restart the shell so it takes effect —
/// a preference that waits for the next launch reads as a broken setting.
pub fn set_terminal_shell(state: AppState, value: Option<String>) {
    #[derive(serde::Serialize)]
    struct Args {
        value: Option<String>,
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::terminal::SET_SHELL, &Args { value }).await },
        move |()| {
            close_terminal(state);
            load_shell_info(state);
        },
    );
}

// ─── detached windows ────────────────────────────────────────────────────────

/// Hand this window's file back to the shell and close.
pub fn reattach(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::REATTACH, &args).await },
        |()| {},
    );
}

/// Reopen a file a detached window is handing back.
///
/// Installed by the shell only: a detached window is one file's editor, and
/// reopening somebody else's tab is exactly the project-wide behaviour it is
/// supposed to stay out of.
pub fn watch_reattach(state: AppState) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Deserialize)]
    struct Event {
        payload: String,
    }

    let handler = Closure::wrap(Box::new(move |event: JsValue| {
        if let Ok(event) = serde_wasm_bindgen::from_value::<Event>(event) {
            open_file(state, event.payload);
        }
    }) as Box<dyn FnMut(JsValue)>);
    // Taken before forgetting, because the handle is what `listen` needs and
    // the closure has to outlive this task either way.
    let js = handler.as_ref().clone();
    handler.forget();
    spawn_local(async move {
        let _ = ipc::listen("rusty://reattach", js).await;
    });
}

// ─── chips ───────────────────────────────────────────────────────────────────

/// The part's pins and what the source names, for the editor's pin map.
pub fn load_pin_report(state: AppState) {
    track(
        state,
        ipc::get::<Option<rusty_embed::PinReport>>(cmd::pins::REPORT),
        move |report| state.pins.set(report),
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

// ─── debugging ───────────────────────────────────────────────────────────────

/// Attach the debugger to a target that is already listening, and keep the
/// panel's state fed until the session ends.
///
/// Which binary and which port are the backend's to know. This used to pass
/// them up out of the cached simulation plan — a plan built for a *release*
/// run — so a debug run booted the unoptimised image while gdb read the
/// optimised one and no breakpoint could ever hit.
pub fn debug_start(state: AppState, hardware: bool) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Serialize)]
    struct Args {
        hardware: bool,
    }

    // Generations, exactly as the terminal needed them: a replaced
    // session's late frames must not overwrite the one that replaced it.
    let epoch = state.debug_epoch.get_untracked() + 1;
    state.debug_epoch.set(epoch);
    // Whether this session has been told about the standing breakpoints.
    let placed = RwSignal::new(false);
    // Frame 0's address at the last stop the editor was moved to.
    let stopped_at = RwSignal::new(None::<String>);
    state.debug.set(Some(rusty_dbg::DebugState::default()));
    state.show_dock(crate::state::DockTab::Debug);

    let channel = ipc::Channel::new();
    let on_state = Closure::wrap(Box::new(move |value: JsValue| {
        if state.debug_epoch.get_untracked() != epoch {
            return;
        }
        if let Ok(update) = serde_wasm_bindgen::from_value::<rusty_dbg::DebugState>(value) {
            // The standing list, once there is a session to place it in.
            // Sending before attach would be refused: the backend has no
            // debugger registered until gdb has answered.
            if update.attached && !placed.get_untracked() {
                placed.set(true);
                for (file, line) in state.breakpoints.get_untracked() {
                    send_breakpoint(state, file, line, None);
                }
                // And then run. `-S` froze the CPU at the reset vector so
                // the breakpoints could be placed before a single
                // instruction executed; without this the session sits there
                // for ever showing `?? 0x40000400`, which is what "the
                // breakpoint never hits" looked like. The commands share
                // one stdin, so the placements are already in front of it.
                debug_control(state, "resume");
            }
            // gdb moves a breakpoint to the next line that has code —
            // an optimised build has none on many lines. The dot follows
            // it, as VSCode's does: it must mark where execution will
            // actually stop, not the line the compiler deleted.
            for placed in &update.breakpoints {
                let Some(asked) = placed.requested else {
                    continue;
                };
                if asked == placed.line {
                    continue;
                }
                let (file, landed) = (placed.file.clone(), placed.line);
                state.breakpoints.update(|list| {
                    if let Some(entry) = list.iter_mut().find(|(f, l)| f == &file && *l == asked) {
                        entry.1 = landed;
                    }
                });
            }

            // Landing on the stopped line is the whole point of stopping —
            // but only when it has *newly* stopped. Every update carries the
            // whole stack, so revealing on each one dragged the editor back
            // to frame 0 a microtask after any click that asked for somewhere
            // else: placing a second breakpoint threw you back to the first,
            // and selecting an outer frame looked like it did nothing at all.
            //
            // Cleared while running, so stopping twice in the same place —
            // a breakpoint in a loop — still reveals.
            if update.running {
                stopped_at.set(None);
            } else if let Some(frame) = update.stack.first() {
                let arrived =
                    stopped_at.with_untracked(|last| last.as_deref() != Some(&frame.address));
                if arrived {
                    stopped_at.set(Some(frame.address.clone()));
                    if let (Some(file), Some(line)) = (frame.file.clone(), frame.line) {
                        open_at(state, file, line, 0);
                    }
                }
            }
            state.debug.set(Some(update));
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_state);
    on_state.forget();

    let args = Args { hardware };
    spawn_local(async move {
        let outcome =
            ipc::call_streaming::<_, ()>(cmd::debug::START, &args, "onState", &channel).await;
        if state.debug_epoch.get_untracked() != epoch {
            return;
        }
        if let Err(error) = outcome {
            state.error.set(Some(error));
        }
        state.debug.set(None);
    });
}

/// Toggle a breakpoint. Lines are zero-based, as everywhere.
///
/// The list is the editor's and survives sessions; a live session is told
/// about the change as it happens, and a session starting later is told
/// the whole list.
pub fn debug_breakpoint(state: AppState, file: String, line: u32) {
    let existed = state
        .breakpoints
        .with_untracked(|list| list.iter().any(|(f, l)| f == &file && *l == line));
    state.breakpoints.update(|list| {
        if existed {
            list.retain(|(f, l)| !(f == &file && *l == line));
        } else {
            list.push((file.clone(), line));
        }
    });

    // Nothing to tell gdb about if gdb is not running — the list is the
    // record, and it will be sent when a session starts.
    if state.debug.with_untracked(Option::is_none) {
        return;
    }
    let number = state.debug.with_untracked(|debug| {
        debug.as_ref().and_then(|debug| {
            debug
                .breakpoints
                .iter()
                .find(|b| b.file == file && b.line == line)
                .and_then(|b| b.number)
        })
    });
    send_breakpoint(state, file, line, existed.then_some(number).flatten());
}

/// One breakpoint over the wire: placing when `remove` is absent, clearing
/// by gdb's own number when it is.
fn send_breakpoint(state: AppState, file: String, line: u32, remove: Option<u32>) {
    #[derive(serde::Serialize)]
    struct Args {
        file: String,
        line: u32,
        remove: Option<u32>,
    }
    let args = Args { file, line, remove };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::debug::BREAKPOINT, &args).await },
        move |()| {},
    );
}

/// Resume, pause, or step.
pub fn debug_control(state: AppState, action: &'static str) {
    #[derive(serde::Serialize)]
    struct Args {
        action: &'static str,
    }
    let args = Args { action };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::debug::CONTROL, &args).await },
        move |()| {},
    );
}

/// Select a stack frame and read its variables.
pub fn debug_frame(state: AppState, level: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        level: u32,
    }
    let args = Args { level };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::debug::FRAME, &args).await },
        move |()| {},
    );
}

/// End the session.
pub fn debug_stop(state: AppState) {
    state.debug_epoch.update(|epoch| *epoch += 1);
    state.debug.set(None);
    spawn_local(async move {
        let _ = ipc::get::<serde_json::Value>(cmd::debug::STOP).await;
    });
}

/// Write the C-interop scaffolding, then say what landed and run the one
/// command it needs. The written files go to the dock, because a workbench
/// that creates four files without naming them is a workbench you stop
/// trusting with your project.
pub fn scaffold_c_interop(state: AppState, direction: &'static str) {
    #[derive(serde::Serialize)]
    struct Args {
        direction: &'static str,
    }
    let args = Args { direction };
    track(
        state,
        async move { ipc::call::<_, rusty_embed::ScaffoldReport>(cmd::wizard::C_INTEROP, &args).await },
        move |report| {
            state.show_dock(crate::state::DockTab::Output);
            for path in &report.written {
                state.push_log(rusty_embed::LogLine {
                    stream: rusty_embed::LogStream::Stdout,
                    text: format!("wrote {path}"),
                    level: None,
                });
            }
            state.push_log(rusty_embed::LogLine {
                stream: rusty_embed::LogStream::Stdout,
                text: format!("next: {}", report.next),
                level: None,
            });
            refresh_tree(state);
            if let Some(command) = report.command {
                run_session(state, command, "build");
            }
        },
    );
}

/// Read the chip's SVD, if this machine has one.
pub fn load_registers(state: AppState) {
    track(
        state,
        async move { ipc::get::<Option<rusty_embed::RegisterMap>>(cmd::debug::REGISTERS).await },
        move |map| state.registers.set(Some(map)),
    );
}

/// Fetch the chip's SVD, then read it. The download's progress goes to the
/// dock like every other download's.
pub fn fetch_svd(state: AppState) {
    let channel = stream_to_terminal(state);
    state.show_dock(crate::state::DockTab::Output);
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, ()>(cmd::debug::FETCH_SVD, &(), "onLine", &channel).await
        },
        move |()| load_registers(state),
    );
}

/// Read a peripheral's register block from the target — one request for the
/// whole span rather than a round trip per register.
pub fn read_peripheral(state: AppState, base: u64, bytes: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        address: u64,
        bytes: u32,
    }
    let args = Args {
        address: base,
        bytes,
    };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::debug::READ, &args).await },
        move |()| {},
    );
}

/// Ask GitHub whether there is a newer rusty.
pub fn check_update(state: AppState) {
    state.update_status.set(None);
    track(
        state,
        async move { ipc::get::<rusty_embed::UpdateStatus>(cmd::workbench::UPDATE).await },
        move |status| state.update_status.set(Some(status)),
    );
}

/// Hand a link to the desktop browser.
pub fn open_url(state: AppState, url: String) {
    #[derive(serde::Serialize)]
    struct Args {
        url: String,
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::OPEN_URL, &Args { url }).await },
        move |()| {},
    );
}

/// The stored shortcut overrides.
pub fn load_keybinds(state: AppState) {
    track(
        state,
        async move {
            ipc::call::<_, std::collections::HashMap<String, String>>(cmd::workbench::KEYBINDS, &())
                .await
        },
        move |map| state.keybinds.set(map),
    );
}

/// Override one shortcut (or clear the override with `None`). Optimistic:
/// the map updates now, the file catches up.
pub fn save_keybind(state: AppState, id: String, chord: Option<String>) {
    state.keybinds.update(|map| match &chord {
        Some(chord) => {
            map.insert(id.clone(), chord.clone());
        }
        None => {
            map.remove(&id);
        }
    });

    #[derive(serde::Serialize)]
    struct Args {
        id: String,
        chord: Option<String>,
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::SET_KEYBIND, &Args { id, chord }).await },
        |()| {},
    );
}

/// Re-read the project tree.
pub fn refresh_tree(state: AppState) {
    if !state.has_project() {
        return;
    }
    track(
        state,
        async move { ipc::call::<_, Vec<Entry>>(cmd::files::TREE, &()).await },
        move |entries| state.file_tree.set(entries),
    );
}

/// Open a file for reading and editing.
///
/// Already on screen: nothing happens — a re-read here would replace an
/// unsaved draft with the disk's older text, which is how editors eat work.
/// Parked: the tab is fronted with its draft intact. New: fetched, and
/// whatever was on screen is parked.
pub fn open_file(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    let active = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
    if active.as_deref() == Some(path.as_str()) {
        return;
    }
    if state
        .parked
        .with_untracked(|parked| parked.iter().any(|e| e.document.path == path))
    {
        activate_tab(state, path);
        return;
    }

    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, Document>(cmd::files::OPEN, &args).await },
        move |document| show_document(state, document, true),
    );
}

/// Re-read the active document from disk and replace it in place — the tail
/// of a save, where disk and draft have just been made equal.
fn reload_active(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, Document>(cmd::files::OPEN, &args).await },
        move |document| show_document(state, document, true),
    );
}

/// Put a freshly loaded document on screen.
///
/// A different path parks the current editor first; the same path replaces it
/// in place, which is how a save's re-read lands without disturbing the strip.
fn show_document(state: AppState, document: Document, announce: bool) {
    let active = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
    if active.is_some() && active.as_deref() != Some(document.path.as_str()) {
        park_active(state);
    }
    if active.as_deref() != Some(document.path.as_str()) {
        state.history.set(EditHistory::default());
    }
    state.tabs.update(|tabs| {
        if !tabs.iter().any(|t| t == &document.path) {
            tabs.push(document.path.clone());
        }
    });
    // Any parked copy is staler than what was just fetched.
    state
        .parked
        .update(|parked| parked.retain(|e| e.document.path != document.path));
    clear_editor_transients(state);
    // The draft is seeded from the document exactly once, here. Setting it
    // anywhere else would overwrite whatever had been typed.
    state.draft.set(document.text.clone());
    state.echo_text.set(document.text.clone());
    state.highlighted.set(document.lines.clone());
    if announce && !document.read_only && state.lsp_status.get_untracked() == LspStatus::Ready {
        lsp_open_doc(document.path.clone(), document.text.clone());
        request_semantic(state, document.path.clone());
    }
    state.document.set(Some(document));
}

/// Stash the on-screen editor into the parked set, caret and all.
fn park_active(state: AppState) {
    let Some(document) = state.document.get_untracked() else {
        return;
    };
    let entry = ParkedEditor {
        draft: state.draft.get_untracked(),
        highlighted: state.highlighted.get_untracked(),
        caret: active_caret(state),
        history: state.history.get_untracked(),
        document,
    };
    state.parked.update(|parked| {
        parked.retain(|e| e.document.path != entry.document.path);
        parked.push(entry);
    });
}

/// The active editor's caret as (line, scalar column), read off the DOM.
///
/// The controller reaching into the DOM is unusual, but the alternative is
/// threading a caret through every caller of every function that might park —
/// and the editor's textarea is as much a singleton as the signals are.
fn active_caret(state: AppState) -> Option<(u32, u32)> {
    use wasm_bindgen::JsCast;
    let element = web_sys::window()?
        .document()?
        .get_element_by_id("editor-area")?
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .ok()?;
    let units = element.selection_start().ok().flatten()? as usize;
    let text = state.draft.get_untracked();
    let mut seen = 0usize;
    let mut line = 0u32;
    let mut col = 0u32;
    for ch in text.chars() {
        if seen >= units {
            break;
        }
        seen += ch.len_utf16();
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    Some((line, col))
}

fn clear_editor_transients(state: AppState) {
    state.completion.set(None);
    state.signature.set(None);
    state.hover.set(None);
    state.semantic.set(None);
    state.actions.set(None);
}

/// Front an already open tab, parking the current one.
pub fn activate_tab(state: AppState, path: String) {
    let active = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
    if active.as_deref() == Some(path.as_str()) {
        return;
    }
    park_active(state);
    if !front_parked(state, &path) {
        // A strip entry with no parked body should not exist; refusing to
        // guess beats showing a stale document as if it were current.
        state.tabs.update(|tabs| tabs.retain(|t| t != &path));
    }
}

/// Move a parked editor onto the screen. False when no such entry exists.
fn front_parked(state: AppState, path: &str) -> bool {
    let mut taken = None;
    state.parked.update(|parked| {
        if let Some(at) = parked.iter().position(|e| e.document.path == path) {
            taken = Some(parked.remove(at));
        }
    });
    let Some(entry) = taken else {
        return false;
    };
    clear_editor_transients(state);
    let dirty = entry.draft != entry.document.text;
    let read_only = entry.document.read_only;
    state.history.set(entry.history);
    state.draft.set(entry.draft.clone());
    state.echo_text.set(entry.draft);
    state.highlighted.set(entry.highlighted);
    state.document.set(Some(entry.document));
    if let Some((line, col)) = entry.caret {
        state.reveal.set(Some(rusty_lsp::Location {
            path: path.to_string(),
            line,
            col,
            external: false,
        }));
    }
    // An edited draft's parked highlight may be a pulse behind; freshen it.
    // Clean or read-only tabs have nothing to freshen.
    if dirty && !read_only {
        schedule_pulse(state);
    }
    if !read_only {
        request_semantic(state, path.to_string());
    }
    true
}

/// Close a tab. Discarding unsaved work requires saying so first.
pub fn close_tab(state: AppState, path: String) {
    let active = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
    let is_active = active.as_deref() == Some(path.as_str());

    let dirty = if is_active {
        state.document.with_untracked(|d| {
            d.as_ref().is_some_and(|d| {
                !d.read_only && state.draft.with_untracked(|draft| draft != &d.text)
            })
        })
    } else {
        state.parked.with_untracked(|parked| {
            parked
                .iter()
                .find(|e| e.document.path == path)
                .is_some_and(|e| !e.document.read_only && e.draft != e.document.text)
        })
    };
    if dirty {
        let confirmed = web_sys::window()
            .map(|w| {
                w.confirm_with_message(&format!(
                    "{path} has unsaved changes.\nClose the tab and discard them?"
                ))
                .unwrap_or(false)
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }
    }

    let next = if is_active {
        neighbour_after_close(&state.tabs.get_untracked(), &path)
    } else {
        None
    };
    state.tabs.update(|tabs| tabs.retain(|t| t != &path));
    state
        .parked
        .update(|parked| parked.retain(|e| e.document.path != path));

    if is_active {
        clear_editor_transients(state);
        let fronted = next.is_some_and(|n| front_parked(state, &n));
        if !fronted {
            state.document.set(None);
            state.draft.set(String::new());
            state.echo_text.set(String::new());
            state.highlighted.set(Vec::new());
            state.history.set(EditHistory::default());
        }
    }
}

/// Which tab takes the screen when this one closes: the one after it, else
/// the one before, else nothing.
fn neighbour_after_close(tabs: &[String], closing: &str) -> Option<String> {
    let at = tabs.iter().position(|t| t == closing)?;
    tabs.get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|i| tabs.get(i)))
        .cloned()
}

/// Open a dependency's source read-only — where goto-definition lands when the
/// answer lives in esp-hal or `core`.
pub fn open_external(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    let active = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
    if active.as_deref() == Some(path.as_str()) {
        return;
    }
    if state
        .parked
        .with_untracked(|parked| parked.iter().any(|e| e.document.path == path))
    {
        activate_tab(state, path);
        return;
    }

    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, Document>(cmd::files::OPEN_EXTERNAL, &args).await },
        // announce=false, deliberately: the server already knows this file as
        // part of the sysroot or a dependency, and announcing it as an
        // editable document would be a lie the read-only flag exists to
        // prevent.
        move |document| show_document(state, document, false),
    );
}

/// Write the current draft back.
pub fn save_file(state: AppState) {
    // A dependency's source is not this project's to change; the backend would
    // refuse the path anyway, but a red banner for pressing Ctrl+S in a file
    // that *looks* editable would blame the user for our affordance.
    if state
        .document
        .with_untracked(|d| d.as_ref().is_some_and(|d| d.read_only))
    {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }

    let Some(path) = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()))
    else {
        return;
    };
    let args = Args {
        path: path.clone(),
        text: state.draft.get_untracked(),
    };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::SAVE, &args).await },
        move |()| {
            lsp_saved_doc(path.clone());
            // Re-read so the highlighting matches what is now on disk, and so
            // the saved/unsaved marker clears against real content rather than
            // against an assumption that the write did what was asked.
            reload_active(state, path.clone());
        },
    );
}

/// Format with rustfmt, then save.
///
/// A rustfmt failure — usually a parse error mid-edit — never blocks the
/// save; the reason goes to the dock instead. `apply` is the editor's own
/// hand: it re-echoes the text and puts the caret back, because the DOM
/// element lives with the view, not here.
pub fn format_then_save(
    state: AppState,
    caret: Option<(u32, u32)>,
    apply: impl Fn(&str, Option<(u32, u32)>) + 'static,
) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }

    let Some(document) = state.document.with_untracked(Clone::clone) else {
        return;
    };
    if document.read_only {
        return;
    }
    let is_rust = document.language.as_deref() == Some("rust") || document.path.ends_with(".rs");
    if !is_rust {
        save_file(state);
        return;
    }

    let args = Args {
        path: document.path,
        text: state.draft.get_untracked(),
    };
    spawn_local(async move {
        match ipc::call::<_, rusty_edit::Formatted>(cmd::files::FORMAT, &args).await {
            Ok(formatted) if formatted.changed => {
                state.draft.set(formatted.text.clone());
                apply(&formatted.text, caret);
            }
            Ok(_) => {}
            Err(error) => {
                // The save below still happens — an unformatted save is a
                // save; a blocked one is data loss waiting for a fix.
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("rustfmt skipped this save: {}", error.message),
                    level: Some(LogLevel::Warn),
                });
            }
        }
        save_file(state);
    });
}

// ─── the language server ─────────────────────────────────────────────────────

/// Start rust-analyzer for the open project and route what it says into state.
pub fn start_lsp(state: AppState) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    if !state.has_project() {
        return;
    }
    // A stale channel keeps sending after a restart; the session number is how
    // its events are told apart from the live one.
    let session = state.lsp_session.get_untracked() + 1;
    state.lsp_session.set(session);
    state.lsp_status.set(LspStatus::Starting);

    let channel = ipc::Channel::new();
    let on_event = Closure::wrap(Box::new(move |value: JsValue| {
        if state.lsp_session.get_untracked() != session {
            return;
        }
        if let Ok(event) = serde_wasm_bindgen::from_value::<LspEvent>(value) {
            apply_lsp_event(state, event);
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_event);
    on_event.forget();

    #[derive(serde::Serialize)]
    struct Args {}

    spawn_local(async move {
        let _ = ipc::call_streaming::<_, ()>(cmd::lsp::START, &Args {}, "onEvent", &channel).await;
        // The stream ended: the server exited or was replaced. Only the owner
        // of the current session gets to say so.
        if state.lsp_session.get_untracked() == session
            && state.lsp_status.get_untracked() == LspStatus::Ready
        {
            state.lsp_status.set(LspStatus::Off);
        }
    });
}

fn apply_lsp_event(state: AppState, event: LspEvent) {
    match event {
        LspEvent::Ready {} => {
            state.lsp_status.set(LspStatus::Ready);
            // A file opened before the server came up was never announced.
            if let Some(path) = state
                .document
                .with_untracked(|d| d.as_ref().map(|d| d.path.clone()))
            {
                lsp_open_doc(path.clone(), state.draft.get_untracked());
                request_semantic(state, path);
            }
        }
        LspEvent::Unavailable { message, install } => {
            state.lsp_status.set(LspStatus::Missing);
            state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: message,
                level: Some(LogLevel::Warn),
            });
            if let Some(install) = install {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: format!("$ {install}"),
                    level: None,
                });
            }
        }
        LspEvent::Diagnostics { path, items } => {
            state.diagnostics.update(|by_file| {
                if items.is_empty() {
                    by_file.remove(&path);
                } else {
                    by_file.insert(path, items);
                }
            });
        }
        LspEvent::Exited {} => {
            if state.lsp_status.get_untracked() == LspStatus::Ready {
                state.lsp_status.set(LspStatus::Off);
            }
        }
    }
}

/// Fire-and-forget document sync. Failures are dropped, not bannered: the
/// editor works without a server, and every keystroke would otherwise be a
/// chance to cry wolf.
fn lsp_sync(command: &'static str, args: impl serde::Serialize + 'static) {
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(command, &args).await;
    });
}

pub fn lsp_open_doc(path: String, text: String) {
    // rust-analyzer is only ever told about Rust. Announcing `.git/info/
    // exclude` as a document got every line a "Syntax Error: expected an
    // item" — sixty-eight problems from a file that was never code.
    if !path.ends_with(".rs") {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }
    lsp_sync(cmd::lsp::OPEN, Args { path, text });
}

fn lsp_saved_doc(path: String) {
    if !path.ends_with(".rs") {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    lsp_sync(cmd::lsp::SAVED, Args { path });
}

/// Ask what the thing at this position is, for the tooltip.
///
/// Silent on failure and on `None`: hover is ambient, and a banner about a
/// hover would be absurd. The reply is dropped if the user has moved to
/// another file by the time it lands.
pub fn request_hover(state: AppState, path: String, line: u32, col: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        line: u32,
        col: u32,
    }

    let args = Args {
        path: path.clone(),
        line,
        col,
    };
    spawn_local(async move {
        if let Ok(Some(info)) = ipc::call::<_, Option<HoverInfo>>(cmd::lsp::HOVER, &args).await {
            let current = state
                .document
                .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
            if current.as_deref() == Some(path.as_str()) {
                // No range from the server means "just this cell" — the card
                // still needs one to decide what counts as moving away.
                let range = info.range.unwrap_or(rusty_lsp::EditRange {
                    start_line: line,
                    start_col: col,
                    end_line: line,
                    end_col: col + 1,
                });
                state.hover.set(Some((path, range, info.text)));
            }
        }
    });
}

/// Jump to wherever the thing at this position is defined.
///
/// The target lands in `state.reveal`; if it is in another file, that file is
/// opened first and the editor applies the reveal once the document arrives.
pub fn goto_definition(state: AppState, path: String, line: u32, col: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        line: u32,
        col: u32,
    }

    let args = Args { path, line, col };
    spawn_local(async move {
        // "No definition" is a normal answer over whitespace or a keyword, and
        // an error here is the server warming up. Neither is worth a banner.
        if let Ok(Some(location)) =
            ipc::call::<_, Option<rusty_lsp::Location>>(cmd::lsp::DEFINITION, &args).await
        {
            let current = state
                .document
                .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
            if current.as_deref() != Some(location.path.as_str()) {
                if location.external {
                    open_external(state, location.path.clone());
                } else {
                    open_file(state, location.path.clone());
                }
            }
            state.reveal.set(Some(location));
        }
    });
}

/// The debounced follow-up to typing: re-highlight the draft and tell the
/// server what it says now.
///
/// Scheduled rather than immediate — each is a round trip, and per keystroke
/// that would re-highlight every letter of a word nobody finished typing.
pub fn schedule_pulse(state: AppState) {
    let generation = state.pulse_gen.get_untracked() + 1;
    state.pulse_gen.set(generation);
    set_timeout(
        move || {
            if state.pulse_gen.get_untracked() == generation {
                edit_pulse(state, generation);
            }
        },
        std::time::Duration::from_millis(250),
    );
}

fn edit_pulse(state: AppState, generation: u64) {
    let Some(path) = state
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()))
    else {
        return;
    };
    let text = state.draft.get_untracked();

    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }

    if path.ends_with(".rs") && state.lsp_status.get_untracked() == LspStatus::Ready {
        request_semantic(state, path.clone());
        lsp_sync(
            cmd::lsp::CHANGE,
            Args {
                path: path.clone(),
                text: text.clone(),
            },
        );
    }

    let args = Args { path, text };
    spawn_local(async move {
        if let Ok(lines) = ipc::call::<_, Vec<EditLine>>(cmd::files::HIGHLIGHT, &args).await {
            // Typing continued while this was in flight: the reply describes a
            // text that no longer exists, and painting it would visibly revert
            // the newest keystrokes until the next pulse.
            if state.pulse_gen.get_untracked() == generation {
                state.highlighted.set(lines);
            }
        }
    });
}

// ─── the terminal ────────────────────────────────────────────────────────────

/// Open a shell and render whatever it draws.
///
/// Frames arrive on a channel and replace the screen wholesale, because a pty
/// *is* a screen: a progress bar redraws its own line, a prompt redraws itself
/// after every backspace, and appending would turn both into a waterfall.
pub fn open_terminal(state: AppState, cols: u16, rows: u16) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Serialize)]
    struct Args {
        cols: u16,
        rows: u16,
    }

    // This call's generation. Everything below writes only while it is
    // still the current one.
    let epoch = state.terminal_epoch.get_untracked() + 1;
    state.terminal_epoch.set(epoch);

    let channel = ipc::Channel::new();
    let on_frame = Closure::wrap(Box::new(move |value: JsValue| {
        if state.terminal_epoch.get_untracked() != epoch {
            return;
        }
        if let Ok(screen) = serde_wasm_bindgen::from_value::<TermScreen>(value) {
            state.terminal.set(Some(screen));
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_frame);
    // Held by the backend for the life of the shell, which outlives this call.
    on_frame.forget();

    // Deliberately not tracked. This call does not return until the shell
    // exits, so counting it as work in flight pins the status bar to "working"
    // for as long as a terminal is open — which is to say, for ever.
    let args = Args { cols, rows };
    spawn_local(async move {
        let outcome =
            ipc::call_streaming::<_, ()>(cmd::terminal::OPEN, &args, "onFrame", &channel).await;
        // A replaced session's ending is not news: the session that replaced
        // it owns the screen now.
        if state.terminal_epoch.get_untracked() != epoch {
            return;
        }
        if let Err(e) = outcome {
            state.error.set(Some(e));
        }
        // A shell that exited leaves its final screen up — it says "exited"
        // and any key starts a fresh one. Only a session that never produced
        // an exit (the error path) drops to the placeholder.
        if state
            .terminal
            .with_untracked(|t| t.as_ref().is_none_or(|screen| screen.exited.is_none()))
        {
            state.terminal.set(None);
        }
    });
}

/// Send keystrokes to the shell.
///
/// Not tracked: this fires on every keypress, and routing it through the busy
/// indicator would make the whole window flicker while you type.
pub fn terminal_input(state: AppState, bytes: Vec<u8>) {
    #[derive(serde::Serialize)]
    struct Args {
        bytes: Vec<u8>,
    }

    let args = Args { bytes };
    spawn_local(async move {
        if let Err(e) = ipc::call::<_, ()>(cmd::terminal::WRITE, &args).await {
            state.error.set(Some(e));
        }
    });
}

/// Tell the shell the view changed size.
pub fn terminal_resize(cols: u16, rows: u16) {
    #[derive(serde::Serialize)]
    struct Args {
        cols: u16,
        rows: u16,
    }

    let args = Args { cols, rows };
    spawn_local(async move {
        // Silent on failure: resizes fire from a layout observer that neither
        // knows nor cares whether a shell is running.
        let _ = ipc::call::<_, ()>(cmd::terminal::RESIZE, &args).await;
    });
}

/// Move the view through scrollback.
pub fn terminal_scroll(state: AppState, delta: i32) {
    #[derive(serde::Serialize)]
    struct Args {
        delta: i32,
    }

    let args = Args { delta };
    spawn_local(async move {
        // Scrolling changes what is shown without the shell writing anything,
        // so the new screen comes back from the call rather than as a frame.
        if let Ok(screen) = ipc::call::<_, TermScreen>(cmd::terminal::SCROLL, &args).await {
            state.terminal.set(Some(screen));
        }
    });
}

pub fn close_terminal(state: AppState) {
    spawn_local(async move {
        // Await the close, *then* clear the screen. Clearing first fires the
        // view's reopen effect immediately, and the close — still in flight —
        // then landed on the session that had just replaced this one, killing
        // the new shell at birth. That is the blank terminal after switching
        // shells.
        let _ = ipc::get::<serde_json::Value>(cmd::terminal::CLOSE).await;
        state.terminal.set(None);
    });
}

/// Install a tool, then notice that it is installed.
///
/// Separate from [`run_command`] only for the re-probe. Without it the panel
/// that offered the install still says the tool is missing after it succeeds,
/// and the user is left pressing a button that has already done its job.
pub fn install_tool(state: AppState, line: String) {
    run_command_then(state, line, move |code| {
        if matches!(code, Some(0) | None) {
            refresh_toolchain(state);
        }
    });
}

/// Run one command in the project root.
pub fn run_command(state: AppState, line: String) {
    run_command_then(state, line, |_| {});
}

fn run_command_then(state: AppState, line: String, after: impl FnOnce(Option<i32>) + 'static) {
    state.log_source.set("commands");
    let mut parts = line.split_whitespace().map(str::to_string);
    let Some(program) = parts.next() else {
        return;
    };
    let args: Vec<String> = parts.collect();

    #[derive(serde::Serialize)]
    struct Args {
        program: String,
        args: Vec<String>,
    }

    // Echo it first. Without this the output has no header and a scrollback of
    // several runs becomes unreadable.
    state.push_log(LogLine {
        stream: LogStream::Stdout,
        text: format!("$ {line}"),
        level: None,
    });

    let channel = stream_to_terminal(state);
    let args = Args { program, args };
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, Option<i32>>(cmd::terminal::RUN, &args, "onLine", &channel)
                .await
        },
        move |code| {
            note_exit(state, code);
            after(code);
        },
    );
}

/// End the running session. The normal way a monitor finishes, not an error.
pub fn stop_session(state: AppState) {
    track(
        state,
        ipc::get::<serde_json::Value>(cmd::flash::STOP),
        move |_| state.session_running.set(false),
    );
}

pub fn dismiss_error(state: AppState) {
    state.error.set(None);
}

/// Window buttons.
///
/// Failures here are deliberately not surfaced: if minimising fails there is
/// nothing the user can do about it, and a banner about it would be noise on
/// top of a window that did not move.
pub fn window_action(command: &'static str) {
    spawn_local(async move {
        let _ = ipc::get::<serde_json::Value>(command).await;
    });
}

#[cfg(test)]
mod tab_tests {
    use super::neighbour_after_close;

    fn tabs(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn the_next_tab_inherits_the_screen() {
        let strip = tabs(&["a.rs", "b.rs", "c.rs"]);
        assert_eq!(
            neighbour_after_close(&strip, "b.rs").as_deref(),
            Some("c.rs")
        );
    }

    #[test]
    fn the_last_tab_falls_back_to_the_previous() {
        let strip = tabs(&["a.rs", "b.rs"]);
        assert_eq!(
            neighbour_after_close(&strip, "b.rs").as_deref(),
            Some("a.rs")
        );
    }

    #[test]
    fn closing_the_only_tab_leaves_nothing() {
        let strip = tabs(&["a.rs"]);
        assert_eq!(neighbour_after_close(&strip, "a.rs"), None);
    }

    #[test]
    fn closing_a_tab_not_in_the_strip_is_a_no_op() {
        let strip = tabs(&["a.rs"]);
        assert_eq!(neighbour_after_close(&strip, "zz.rs"), None);
    }
}
