//! rusty's frontend.
//!
//! Leptos in CSR mode inside Tauri's WebView. Layered the way velo's is, and
//! for the same reason — the discipline is what keeps a workbench from becoming
//! a pile of components that each fetch their own data:
//!
//! | Layer | Rule |
//! |---|---|
//! | `view` | renders; never calls IPC |
//! | `controller` | the only place a cross-layer action begins |
//! | `state` | signals and pure operations on them |
//! | `ipc` | transport, and nothing else |

mod command;
mod controller;
mod format;
mod i18n;
mod ipc;
mod state;
mod theme;
mod view;
mod vim;

use leptos::prelude::*;

fn main() {
    // Without this a Rust panic in wasm is an unhelpful "unreachable executed"
    // in the console, with no line and no message.
    console_error_panic_hook::set_once();

    // The language, before anything is drawn — the setting lives in
    // `workbench.toml` but arrives over IPC, and a first paint in the wrong
    // language is a visible flicker. So this reads the boot cache, and
    // `restore_locale` reconciles it with the file once the window is up.
    i18n::apply_boot_locale();

    mount_to_body(view::App);
}
