//! The shell, and the one-shot runner the panels use to launch a tool.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::{LogLine, LogStream};
use rusty_term::Screen as TermScreen;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

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
    let epoch = state.term.epoch.get_untracked() + 1;
    state.term.epoch.set(epoch);

    let channel = ipc::Channel::new();
    let on_frame = Closure::wrap(Box::new(move |value: JsValue| {
        if state.term.epoch.get_untracked() != epoch {
            return;
        }
        if let Ok(screen) = serde_wasm_bindgen::from_value::<TermScreen>(value) {
            state.term.screen.set(Some(screen));
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
        if state.term.epoch.get_untracked() != epoch {
            return;
        }
        if let Err(e) = outcome {
            state.app.error.set(Some(e));
        }
        // A shell that exited leaves its final screen up — it says "exited"
        // and any key starts a fresh one. Only a session that never produced
        // an exit (the error path) drops to the placeholder.
        if state
            .term
            .screen
            .with_untracked(|t| t.as_ref().is_none_or(|screen| screen.exited.is_none()))
        {
            state.term.screen.set(None);
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
            state.app.error.set(Some(e));
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
            state.term.screen.set(Some(screen));
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
        state.term.screen.set(None);
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

pub(super) fn run_command_then(
    state: AppState,
    line: String,
    after: impl FnOnce(Option<i32>) + 'static,
) {
    run_command_in(state, line, false, after);
}

/// Run at the opened project rather than at the firmware crate.
///
/// For the host half: `cargo test` in a bare-metal crate cannot link a test
/// harness, and that crate is excluded from the workspace for exactly that
/// reason. See `run_command` on the backend.
pub(super) fn run_command_at_root(state: AppState, line: String) {
    run_command_in(state, line, true, |_| {});
}

/// A command given as one line, split on whitespace — what the palette and
/// the recipes hand over. An argument that must keep its spaces goes through
/// [`run_args_at_root_then`] instead.
fn run_command_in(
    state: AppState,
    line: String,
    at_project_root: bool,
    after: impl FnOnce(Option<i32>) + 'static,
) {
    let mut parts = line.split_whitespace().map(str::to_string);
    let Some(program) = parts.next() else {
        return;
    };
    let args: Vec<String> = parts.collect();
    run_parts_in(state, program, args, at_project_root, after);
}

/// A command given as a program and its arguments, one argument one string
/// however many spaces it holds — a commit message, a stash note. The dock
/// shows it quoted the way a shell would want it typed.
pub(super) fn run_args_at_root_then(
    state: AppState,
    program: impl Into<String>,
    args: Vec<String>,
    after: impl FnOnce(Option<i32>) + 'static,
) {
    run_parts_in(state, program.into(), args, true, after);
}

fn run_parts_in(
    state: AppState,
    program: String,
    args: Vec<String>,
    at_project_root: bool,
    after: impl FnOnce(Option<i32>) + 'static,
) {
    state.dock.source.set("commands");

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        program: String,
        args: Vec<String>,
        at_project_root: bool,
    }

    // Echo it first. Without this the output has no header and a scrollback of
    // several runs becomes unreadable.
    let line = std::iter::once(program.clone())
        .chain(args.iter().map(|arg| shell_word(arg)))
        .collect::<Vec<_>>()
        .join(" ");
    state.push_log(LogLine {
        stream: LogStream::Stdout,
        text: format!("$ {line}"),
        level: None,
    });

    let channel = stream_to_terminal(state);
    let args = Args {
        program,
        args,
        at_project_root,
    };
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

/// How the dock spells an argument: as it is when a shell would read it
/// back as one word, quoted otherwise. Display only — the process gets the
/// argument itself, never this.
fn shell_word(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./:=@+,~".contains(c))
    {
        arg.to_string()
    } else {
        format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// End the running session. The normal way a monitor finishes, not an error.
pub fn stop_session(state: AppState) {
    track(
        state,
        ipc::get::<serde_json::Value>(cmd::flash::STOP),
        move |_| state.app.session_running.set(false),
    );
}

pub fn dismiss_error(state: AppState) {
    state.app.error.set(None);
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

/// Run the tests a filter names, in the dock.
///
/// The filter is a **substring**, not `--exact`, and that is forced rather
/// than chosen: the editor's scan sees the modules inside one file, so it
/// knows `tests::it_works` but not the `foo::bar::` the file itself sits at.
/// `cargo test` with an `--exact` path that matches nothing exits *zero*
/// having run nothing, which on screen is indistinguishable from a pass — so
/// the broader filter is the safe one. Running a same-named test in a sibling
/// module too is a visible extra line of output; a silent green tick is not.
///
/// `--nocapture` because the reason to click one test rather than run the
/// suite is usually to read what it prints.
pub fn run_test(state: AppState, filter: String) {
    let line = if filter.is_empty() {
        "cargo test".to_string()
    } else {
        format!("cargo test {filter} -- --nocapture")
    };
    // At the project, not at the firmware crate: see `run_command_at_root`.
    run_command_at_root(state, line);
}

#[cfg(test)]
mod tests {
    use super::shell_word;

    #[test]
    fn a_word_a_shell_reads_back_whole_is_shown_as_itself() {
        assert_eq!(shell_word("commit"), "commit");
        assert_eq!(shell_word("--include-untracked"), "--include-untracked");
        assert_eq!(shell_word("origin/main"), "origin/main");
        assert_eq!(shell_word("v0.4.0"), "v0.4.0");
    }

    #[test]
    fn anything_a_shell_would_split_or_expand_is_quoted() {
        assert_eq!(shell_word("a b"), "\"a b\"");
        // Braces expand in bash; a stash name has to be quoted to be typed.
        assert_eq!(shell_word("stash@{0}"), "\"stash@{0}\"");
        assert_eq!(shell_word(""), "\"\"");
    }

    #[test]
    fn quotes_and_backslashes_inside_are_escaped() {
        assert_eq!(shell_word("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(shell_word("C:\\path"), "\"C:\\\\path\"");
    }

    #[test]
    fn a_message_with_paragraphs_is_one_quoted_word() {
        let shown = shell_word("first line\n\nsecond paragraph");
        assert!(shown.starts_with('"') && shown.ends_with('"'));
        assert_eq!(shown.matches('"').count(), 2, "one opening, one closing");
        assert!(
            shown.contains("\n\n"),
            "the newlines are shown, not escaped"
        );
    }
}
