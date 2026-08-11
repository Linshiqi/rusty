//! The assistant command.
//!
//! Streaming lives here rather than in `commands.rs` because it is the only
//! command that is long-running, and the shape it needs — a channel out, a
//! conversation back — is unlike everything else.

use rusty_ai::{AgentEvent, Assistant, Message, ProviderConfig, ToolContext, config};
use tauri::{State, ipc::Channel};

use crate::{error::CommandError, state::AppState};

/// Ask the assistant a question, streaming progress as it goes.
///
/// Returns the updated conversation. The frontend keeps the transcript and
/// sends it back next time, so the backend holds no session state — closing a
/// panel cannot strand a conversation, and several can run at once.
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

    let assistant = Assistant::new(config::build(&config)?).with_max_tokens(config.max_tokens);
    let mut history = history;

    assistant
        .ask(&context, &mut history, &mut |event| {
            // A closed channel means the user navigated away mid-answer. That
            // is not an error, and the loop should finish so the transcript
            // stays consistent.
            let _ = on_event.send(event);
        })
        .await?;

    Ok(history)
}
