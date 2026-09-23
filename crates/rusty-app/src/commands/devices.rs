//! The boards plugged in — serial ports and debug probes — and the command
//! a flash or a monitor would run on one. Running it is `crate::flash`'s.

use std::path::PathBuf;

use rusty_embed::{CommandPlan, FlashAction, Probe, SerialPort, Transport, device, flash, project};
use tauri::State;

use super::Answer;
use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

/// Serial ports currently attached, named against the board catalogue and with
/// likely boards first.
#[tauri::command]
pub async fn serial_ports(state: State<'_, AppState>) -> Answer<Vec<SerialPort>> {
    let catalog = state.catalog().await;
    blocking("listing the serial ports", move || {
        device::list_serial_ports(catalog.as_ref())
    })
    .await
}

/// Debug probes, via `probe-rs list`.
///
/// A process that waits on USB enumeration — seconds, on a hub with a few
/// devices — so off the IPC thread, where it used to freeze the window for
/// exactly that long.
#[tauri::command]
pub async fn debug_probes() -> Answer<Vec<Probe>> {
    blocking("listing the probes", device::list_probes).await
}

/// Work out the command without running it.
///
/// The UI shows this before the user commits, and the assistant can quote it.
/// Separating the decision from the execution is also what makes the choice of
/// tool and flags testable without a board attached.
#[tauri::command]
pub async fn plan_flash(
    transport: Transport,
    action: FlashAction,
    // Absent for a monitor attached before anything was built; the planner
    // refuses everything that would need it.
    firmware: Option<String>,
    defmt: bool,
    baud: Option<u32>,
    state: State<'_, AppState>,
) -> Answer<CommandPlan> {
    let root = state.require_firmware_root().await?;
    let catalog = state.catalog().await;
    blocking("planning the flash", move || {
        let chip_id = project::detect(&root)?.chip.ok_or_else(|| {
            CommandError::new(
                "The target chip is unknown, so rusty cannot choose a flashing command. \
                 Fix the problems listed in the Project panel first.",
            )
        })?;
        // Enumerated only when the plan needs it: a probe asks nothing of the
        // serial ports.
        let ports = match &transport {
            Transport::Serial { .. } => device::list_serial_ports(&catalog),
            Transport::Probe { .. } => Vec::new(),
        };
        let warning = flash::port_warning(&chip_id, &transport, &ports, &catalog);

        let mut plan = flash::plan(&flash::FlashRequest {
            chip_id,
            transport,
            action,
            firmware: firmware.map(PathBuf::from),
            defmt,
            baud,
        })?;
        plan.warning = warning;
        Ok(plan)
    })
    .await?
}
