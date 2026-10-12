//! This window and the host it runs on: the platform, the interface zoom,
//! and a detached editor window's way back.

use super::*;

/// Which desktop this window is on, for the words that differ: Explorer or
/// Finder, the Recycle Bin or the Trash.
pub fn host_is_windows() -> bool {
    host_platform().starts_with("Win")
}

pub fn host_is_mac() -> bool {
    host_platform().starts_with("Mac")
}

fn host_platform() -> String {
    web_sys::window()
        .and_then(|w| w.navigator().platform().ok())
        .unwrap_or_default()
}

/// Float a file into its own OS window.
pub fn detach_file(state: AppState, path: String) {
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::DETACH, &PathArg { path }).await },
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
    let factor = state.layout.zoom.get_untracked();
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::window::SET_ZOOM, &Args { factor }).await },
        |()| {},
    );
}

/// Hand this window's file back to the shell and close.
pub fn reattach(state: AppState, path: String) {
    let args = PathArg { path };
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

    // Guarded like every other call that runs at mount. `catch` on an async
    // extern turns a *rejected promise* into `Err` and nothing else: with no
    // `window.__TAURI__`, the shim throws synchronously, the generated glue
    // swallows that and hands back `undefined`, and the wasm side then calls
    // `.then` on it. The uncaught TypeError kills the executor, so the page
    // paints once and answers nothing afterwards — no banner, no clue.
    if !ipc::backend_available() {
        return;
    }

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
