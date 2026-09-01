//! Where rusty keeps its data, and moving it.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::{RelocateReport, StorageLocation};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

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
                        "Moved: {} files copied to {}. The originals are still in {} — \
                         delete them yourself once you are satisfied.",
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
