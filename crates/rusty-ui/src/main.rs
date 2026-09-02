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

#[cfg(test)]
mod hygiene {
    /// `mock.js` stubs the IPC surface so `trunk serve` can exercise backend
    /// flows in a plain browser. The line that wires it into `index.html` is
    /// a debugging aid and must never ship — and it had been committed four
    /// times, each time by somebody who meant to take it out. CI reads the
    /// file so the fifth time fails a test instead of reaching a release.
    #[test]
    fn the_ipc_mock_is_not_wired_into_index_html() {
        let html = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/index.html"))
            .expect("read index.html");
        assert!(
            !html.contains("mock.js"),
            "index.html still loads mock.js — remove the <link>/<script> pair before committing"
        );
    }
}
