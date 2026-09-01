//! Starting a new project.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::{
    CommandPlan, Explanation, LogLevel, LogLine, LogStream, WizardChoice, WizardOption,
};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Load the generator's options. Static, so once per session is enough.
pub fn load_wizard_options(state: AppState) {
    track(
        state,
        ipc::get::<Vec<WizardOption>>(cmd::wizard::OPTIONS),
        move |options| state.wizard.options.set(options),
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

    state.wizard.choice.set(Some(choice.clone()));

    let explain_args = Args {
        choice: choice.clone(),
    };
    track(
        state,
        async move { ipc::call::<_, Vec<Explanation>>(cmd::wizard::EXPLAIN, &explain_args).await },
        move |explanations| state.wizard.explanations.set(explanations),
    );

    // The plan can legitimately fail — a chip with no `std` target under the
    // ESP-IDF runtime, say — and that refusal is the useful answer. It surfaces
    // through the normal error path and clears the stale command.
    state.wizard.plan.set(None);
    let plan_args = Args { choice };
    track(
        state,
        async move { ipc::call::<_, CommandPlan>(cmd::wizard::PLAN, &plan_args).await },
        move |plan| state.wizard.plan.set(Some(plan)),
    );
}

/// Ask where it should go, generate it, then open it.
///
/// The command is still shown — that is what makes the tool inspectable — but
/// showing it *instead* of acting made this panel a slow way to type. The one
/// decision rusty must not make quietly is where the code lands, and that is
/// exactly what the folder picker asks.
pub fn create_project(state: AppState, choice: WizardChoice) {
    state.dock.source.set("tools");
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
                state.app.error.set(Some(e));
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
                state.app.session_running.set(false);
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
                state.layout.panel.set("files".to_string());
            },
        );
    });
}
