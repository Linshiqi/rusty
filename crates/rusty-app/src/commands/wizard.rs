//! A new project: the generator's options and what each commits to, the
//! command that makes it, and C scaffolding for an existing one.

use rusty_embed::{CommandPlan, Explanation, WizardChoice, WizardOption, project, wizard};
use tauri::State;

use super::Answer;
use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

/// Generator options, with what each one costs.
#[tauri::command]
pub fn wizard_options() -> Vec<WizardOption> {
    wizard::options()
}

/// What the current selection commits the user to.
///
/// Called on every change in the wizard, not just at the end: the point is to
/// answer "what does this choice mean" while the choice is still being made.
#[tauri::command]
pub fn explain_choice(choice: WizardChoice) -> Vec<Explanation> {
    wizard::explain(&choice)
}

#[tauri::command]
pub fn plan_new_project(choice: WizardChoice) -> Answer<CommandPlan> {
    Ok(wizard::plan(&choice)?)
}

/// Write the C-interop scaffolding, in whichever direction.
///
/// Returns what it wrote and what still has to run, so the panel can say so
/// rather than leaving somebody to discover a build.rs they did not expect.
#[tauri::command]
pub async fn scaffold_c_interop(
    direction: String,
    state: State<'_, AppState>,
) -> Answer<rusty_embed::ScaffoldReport> {
    use rusty_embed::scaffold::Direction;

    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;

    let direction = match direction.as_str() {
        "rust-calls-c" => Direction::RustCallsC,
        "c-calls-rust" => Direction::CCallsRust,
        other => {
            return Err(CommandError::new(format!(
                "{other} is not a direction rusty can scaffold",
            )));
        }
    };

    blocking("scaffolding", move || {
        // Before anything is written — see `c_compiler_gate`.
        let detected = project::detect(&root)?;
        let chip = detected.chip.as_deref().and_then(rusty_embed::chip::by_id);
        rusty_embed::scaffold::c_compiler_gate(chip.as_ref(), |binary| {
            rusty_embed::tools::find(binary).is_some()
        })
        .map_err(CommandError::new)?;

        let scaffold = rusty_embed::scaffold::c_interop(&root, direction)
            .map_err(|e| CommandError::new(e.to_string()))?;
        Ok(rusty_embed::ScaffoldReport {
            written: scaffold.written,
            command: scaffold.command,
            next: scaffold.next,
        })
    })
    .await?
}
