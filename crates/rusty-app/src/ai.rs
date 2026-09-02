//! The assistant command.
//!
//! Streaming lives here rather than in `commands.rs` because it is the only
//! command that is long-running, and the shape it needs — a channel out, a
//! conversation back — is unlike everything else.

use std::time::Duration;

use rusty_ai::{AgentEvent, Assistant, Message, ProviderConfig, ToolContext, config};
use tauri::{State, ipc::Channel};

use crate::{
    commands::ai_inputs,
    error::CommandError,
    state::{AppState, blocking},
};

/// Longest one question may run, tool rounds included.
///
/// Generous — eight rounds of a slow model, each with its own network
/// timeouts underneath — but finite. Nothing else bounds the loop, and a
/// question that never resolves is a panel that never stops spinning.
const ASK_BUDGET: Duration = Duration::from_secs(20 * 60);

/// Ask the assistant a question, streaming progress as it goes.
///
/// Returns the updated conversation. The frontend keeps the transcript and
/// sends it back next time, so the backend holds no session state — closing a
/// panel cannot strand a conversation.
///
/// One question at a time, and it can be stopped: it is registered in
/// `AppState` on the way in, [`ai_cancel`] stops it, and so does asking again
/// before it has finished. A stopped question resolves with
/// [`CommandError::cancelled`] rather than a transcript, because the loop was
/// interrupted and whatever it had accumulated is not what the model said.
/// Until this existed a closed panel left the loop running for up to eight
/// more tool rounds, on the user's key, with nothing able to reach it — a
/// channel whose JavaScript side is gone tells the Rust side nothing.
///
/// The open-project handles are cloned out of the lock before the first token,
/// so a slow model never blocks the rest of the window.
#[tauri::command]
pub async fn ai_ask(
    config: ProviderConfig,
    history: Vec<Message>,
    on_event: Channel<AgentEvent>,
    state: State<'_, AppState>,
) -> Result<Vec<Message>, CommandError> {
    let open = state.snapshot().await;

    // Deliberately permissive: the assistant is useful with nothing open at
    // all — "which ESP32 has 802.15.4?" needs no project — and each tool
    // reports for itself what it is missing.
    let catalog = state.catalog().await;
    let context = ToolContext {
        workspace: open.workspace.as_deref(),
        root: open.root(),
        firmware: open.firmware.clone(),
        catalog: Some(&catalog),
    };

    // The key and the proxy are the machine's to answer, off the async thread.
    let (key, http) = ai_inputs(config.profile.clone()).await?;
    let provider = {
        let settings = config.clone();
        blocking("building the provider", move || {
            config::build(&settings, key, &http)
        })
        .await??
    };
    let assistant = Assistant::new(provider).with_max_tokens(config.max_tokens);
    let mut history = history;

    let (ticket, stop) = state.begin_ask().await;
    let outcome = {
        // A failed send means the WebView is gone, not that the user
        // navigated away; either way the loop's end is the token's job,
        // not this callback's. Bound to a name so it outlives the future
        // that borrows it across the `select!`.
        let mut deliver = |event| {
            let _ = on_event.send(event);
        };
        let question = assistant.ask(&context, &mut history, &mut deliver);
        tokio::select! {
            finished = tokio::time::timeout(ASK_BUDGET, question) => match finished {
                Ok(result) => result.map_err(CommandError::from),
                Err(_) => Err(CommandError::new(format!(
                    "The assistant did not finish within {} minutes and was stopped.",
                    ASK_BUDGET.as_secs() / 60,
                ))),
            },
            () = stop.cancelled() => Err(CommandError::cancelled()),
        }
    };
    state.end_ask(ticket).await;

    outcome.map(|()| history)
}

/// Stop the question in flight, if any. `ai_ask` then resolves with
/// [`CommandError::cancelled`].
#[tauri::command]
pub async fn ai_cancel(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.cancel_ask().await;
    Ok(())
}
