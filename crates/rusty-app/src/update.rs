//! Is there a newer rusty, and getting it.
//!
//! Four steps, each its own command, because the shape of the thing is a
//! hundred-megabyte download that ends by restarting the app:
//!
//! - **check** asks the release feed through the Tauri updater and answers
//!   with what it found — version, date, notes — as an [`UpdateStatus`]. The
//!   `Update` itself stays in [`AppState`] for the step after.
//! - **download** fetches the installer, streaming progress, and verifies
//!   its signature against the public key compiled into this build before a
//!   byte of it is kept. Cancellable: the fetch runs as a task whose abort
//!   handle the state holds.
//! - **apply** runs what was downloaded and restarts. Separate from the
//!   download on purpose: the restart is the user's gesture, taken when they
//!   are ready, not the download's end arriving while they type.
//! - **skip** remembers a version not to prompt about again.
//!
//! Checking is the only step with a quiet failure mode. No network is the
//! normal state of a workbench on a bench, so an unreachable feed is a note
//! in the answer rather than an error, and the automatic check at launch
//! shows nothing for it.

use std::time::Duration;

use rusty_embed::{UpdateProgress, UpdateStatus, config as storage};
use tauri::{AppHandle, State, ipc::Channel};
use tauri_plugin_updater::UpdaterExt;

use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

type Answer<T> = Result<T, CommandError>;

/// How often the download reports: once per this many bytes, and at the end.
/// A hundred megabytes arrive in sixteen-kilobyte chunks, and an IPC message
/// per chunk would be six thousand of them for one progress bar.
const PROGRESS_STEP: u64 = 256 * 1024;

/// The feed to check in a development build, instead of the configured one
/// — how the whole flow is exercised against a local server before a release
/// exists to test it against. Release builds never read it, and the plugin
/// refuses a plain-http endpoint in them regardless.
const DEV_FEED: &str = "RUSTY_UPDATE_FEED";

/// Where the updater's requests go, from the same setting every other fetch
/// reads.
enum Route {
    /// Let the client find the system's own proxy, if any.
    System,
    /// The user said "none": straight out, whatever the environment says.
    Direct,
    /// An explicit or detected proxy.
    Through(url::Url),
}

fn route(setting: Option<&str>) -> Route {
    if setting
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("none"))
    {
        return Route::Direct;
    }
    match rusty_embed::net::effective_proxy().and_then(|proxy| proxy.parse().ok()) {
        Some(url) => Route::Through(url),
        None => Route::System,
    }
}

/// Is there a newer rusty?
///
/// The answer says whether the user asked to skip the version it names;
/// whether to interrupt them is the frontend's decision — the launch check
/// stays quiet, a check asked for by hand shows it anyway.
#[tauri::command]
pub async fn check_update(app: AppHandle, state: State<'_, AppState>) -> Answer<UpdateStatus> {
    let current = app.package_info().version.to_string();
    // The proxy setting, and on Windows a registry query behind it: blocking
    // work like every other read of the machine.
    let (proxy, skipped) = blocking("reading the proxy setting", || {
        let workbench = storage::workbench();
        (route(workbench.proxy.as_deref()), workbench.skipped_update)
    })
    .await?;

    let mut builder = app.updater_builder().timeout(Duration::from_secs(30));
    builder = match proxy {
        Route::Direct => builder.no_proxy(),
        Route::Through(url) => builder.proxy(url),
        Route::System => builder,
    };
    if cfg!(debug_assertions)
        && let Ok(feed) = std::env::var(DEV_FEED)
    {
        let url = feed
            .parse()
            .map_err(|e| CommandError::new(format!("{DEV_FEED} is not a URL: {e}")))?;
        builder = builder.endpoints(vec![url])?;
    }
    let updater = builder.build()?;

    let idle = |current: String, note: Option<String>| UpdateStatus {
        current,
        latest: None,
        url: None,
        newer: false,
        note,
        notes: None,
        date: None,
        skipped: false,
    };

    let status = match updater.check().await {
        Ok(Some(update)) => {
            let status = UpdateStatus {
                current,
                latest: Some(update.version.clone()),
                url: Some(format!(
                    "{}/tag/v{}",
                    rusty_embed::REPO_RELEASES,
                    update.version
                )),
                newer: true,
                note: None,
                notes: update.body.clone().filter(|notes| !notes.trim().is_empty()),
                date: update.date.map(|date| date.date().to_string()),
                skipped: skipped.as_deref() == Some(update.version.as_str()),
            };
            state.hold_update(Some(update)).await;
            status
        }
        Ok(None) => {
            state.hold_update(None).await;
            idle(current, None)
        }
        Err(error) => idle(current, Some(error.to_string())),
    };
    Ok(status)
}

/// Fetch the update the last check found, verifying its signature.
///
/// `true` when the installer is held and verified, `false` when the download
/// was cancelled — an answer, not a failure. A signature that does not verify
/// is a failure, in the plugin's own words.
#[tauri::command]
pub async fn download_update(
    on_progress: Channel<UpdateProgress>,
    state: State<'_, AppState>,
) -> Answer<bool> {
    let Some(update) = state.pending_update().await else {
        return Err(CommandError::new(
            "No update has been found to download — check for updates first.",
        ));
    };

    let task = tokio::spawn(async move {
        let mut received = 0u64;
        let mut reported = 0u64;
        update
            .download(
                |chunk, total| {
                    received += chunk as u64;
                    if received - reported >= PROGRESS_STEP || Some(received) == total {
                        reported = received;
                        let _ = on_progress.send(UpdateProgress { received, total });
                    }
                },
                || {},
            )
            .await
    });
    // A download already in flight is replaced, not raced: two copies of the
    // installer arriving at once would both be held by nobody.
    if let Some(previous) = state.set_downloading(Some(task.abort_handle())).await {
        previous.abort();
    }
    let outcome = task.await;
    state.set_downloading(None).await;

    match outcome {
        Ok(Ok(bytes)) => {
            state.hold_download(Some(bytes)).await;
            Ok(true)
        }
        Ok(Err(error)) => Err(error.into()),
        Err(join) if join.is_cancelled() => Ok(false),
        Err(join) => Err(CommandError::new(format!(
            "the update download panicked: {join}"
        ))),
    }
}

/// Stop a download in flight. Nothing is kept of it.
#[tauri::command]
pub async fn cancel_update(state: State<'_, AppState>) -> Answer<()> {
    if let Some(task) = state.set_downloading(None).await {
        task.abort();
    }
    Ok(())
}

/// Install what was downloaded, and restart into it.
///
/// On Windows the installer is handed to the shell and this process exits
/// from inside `install`; elsewhere the bundle is replaced in place and the
/// restart below is what picks the new one up.
#[tauri::command]
pub async fn apply_update(app: AppHandle, state: State<'_, AppState>) -> Answer<()> {
    let (Some(update), Some(bytes)) = (state.pending_update().await, state.take_download().await)
    else {
        return Err(CommandError::new(
            "Nothing has been downloaded to install — download the update first.",
        ));
    };
    blocking("installing the update", move || update.install(bytes)).await??;
    app.restart()
}

/// Stop prompting about this version. A check asked for by hand still
/// reports it, and the next release is offered as usual.
#[tauri::command]
pub async fn skip_update(version: String, state: State<'_, AppState>) -> Answer<()> {
    state
        .update_workbench(move |workbench| workbench.skipped_update = Some(version))
        .await
}
