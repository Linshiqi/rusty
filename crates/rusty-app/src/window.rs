//! Window controls.
//!
//! The OS title bar is off (`decorations: false`), so the app draws its own.
//! That is one row instead of two, and it is where every desktop application
//! that cares about its chrome has ended up.
//!
//! Dragging is handled by `data-tauri-drag-region` in the frontend rather than
//! from here; only the buttons need to reach the backend.
//!
//! The `Window` argument is injected by Tauri and is the window that made the
//! call — no lookup by label, which would only be able to get it wrong.

use std::ffi::OsString;
use std::path::PathBuf;

use tauri::{State, Window};

use crate::error::CommandError;

fn fail(e: tauri::Error) -> CommandError {
    CommandError::new(e.to_string())
}

/// What New Window starts this app with.
pub const NEW_WINDOW: &str = "--new-window";

/// This instance was started by New Window, and opens on the welcome screen
/// rather than on the project the last session had: that project is open in
/// the window it was asked from.
pub struct Fresh(pub bool);

#[tauri::command]
pub fn window_fresh(fresh: State<'_, Fresh>) -> bool {
    fresh.0
}

/// New Window: another rusty, with a project of its own.
///
/// **Another instance, not another window of this one.** The backend holds
/// one project — its language server, its watcher, its sessions and the
/// emulator all belong to it — and a window here shares them, which is what
/// the detached editor and the commit window are for. A second project is a
/// second process, as a second VS Code window is a second renderer.
///
/// The child is nobody's to wait on, so a thread reaps it — on Unix one left
/// unwaited is a zombie until this instance exits — and it starts in a
/// process group of its own, so a Ctrl+C in the terminal that ran the first
/// does not end the second.
#[tauri::command]
pub fn window_new() -> Result<(), CommandError> {
    let exe = std::env::current_exe()
        .map_err(|e| CommandError::new(format!("could not find rusty's own executable: {e}")))?;
    let program = relaunch(exe, std::env::var_os("APPIMAGE"));
    let mut command = std::process::Command::new(&program);
    command
        .arg(NEW_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // A debug build is a console program; without this the new window
        // would come with a console window behind it.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| {
        CommandError::new(format!(
            "could not start another rusty ({}): {e}",
            program.display()
        ))
    })?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// What to start again. From an AppImage, the AppImage: this executable
/// lives in the mount the running image made, which goes when this instance
/// does and would take a second one started from it along. Anything else is
/// this executable — an installed app, a macOS bundle's binary, a dev build.
fn relaunch(exe: PathBuf, appimage: Option<OsString>) -> PathBuf {
    appimage
        .filter(|image| !image.is_empty())
        .map(PathBuf::from)
        .unwrap_or(exe)
}

#[tauri::command]
pub fn window_minimize(window: Window) -> Result<(), CommandError> {
    window.minimize().map_err(fail)
}

/// Toggle, returning the new state so the button can show the right glyph.
#[tauri::command]
pub fn window_toggle_maximize(window: Window) -> Result<bool, CommandError> {
    let maximized = window.is_maximized().map_err(fail)?;
    if maximized {
        window.unmaximize().map_err(fail)?;
    } else {
        window.maximize().map_err(fail)?;
    }
    Ok(!maximized)
}

/// Scale the whole interface, browser-zoom style. CSS pixels stay
/// self-consistent at every factor, so nothing that measures text — the
/// editor's hit-testing above all — drifts.
#[tauri::command]
pub fn window_set_zoom(factor: f64, webview: tauri::WebviewWindow) -> Result<(), CommandError> {
    webview.set_zoom(factor.clamp(0.7, 1.6)).map_err(fail)
}

#[tauri::command]
pub fn window_close(window: Window) -> Result<(), CommandError> {
    // `close()` rather than `destroy()`, so a future CloseRequested handler can
    // intervene — an unsaved wizard, or a flash in progress.
    window.close().map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_appimage_starts_the_image_and_everything_else_itself() {
        let exe = PathBuf::from("/tmp/.mount_rustyX/usr/bin/rusty-app");
        assert_eq!(
            relaunch(exe.clone(), Some("/home/me/rusty.AppImage".into())),
            PathBuf::from("/home/me/rusty.AppImage"),
            "the mount goes with the instance that made it"
        );
        assert_eq!(relaunch(exe.clone(), None), exe);
        assert_eq!(relaunch(exe.clone(), Some(OsString::new())), exe);
    }
}
