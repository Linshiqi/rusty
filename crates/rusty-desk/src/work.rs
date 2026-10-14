//! Work off the UI thread, and its answer back on it.
//!
//! The UI thread never waits on the disk, a process, the network or a
//! language server. What would make it wait runs here, on a thread of its
//! own, and the answer is handed to the app on the UI thread through
//! Slint's event loop — the whole of what the Tauri command layer did.

use std::{cell::OnceCell, rc::Rc};

use crate::app::App;

thread_local! {
    static APP: OnceCell<Rc<App>> = const { OnceCell::new() };
}

/// Make `app` the one the event loop hands answers to. Once, at start.
pub fn install(app: Rc<App>) {
    APP.with(|cell| {
        cell.set(app).ok().expect("the app is installed once");
    });
}

/// The app, on the UI thread.
pub fn app() -> Rc<App> {
    APP.with(|cell| cell.get().expect("the app is installed").clone())
}

/// Run `job` on a thread of its own and hand its answer to `then` on the UI
/// thread. A window that has gone by then drops the answer.
pub fn spawn<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&Rc<App>, T) + Send + 'static,
) {
    std::thread::spawn(move || {
        let answer = job();
        let _ = slint::invoke_from_event_loop(move || {
            let app = APP.with(|cell| cell.get().cloned());
            if let Some(app) = app {
                then(&app, answer);
            }
        });
    });
}
