//! The frontend's only door to the backend.
//!
//! Everything goes through [`call`] and its variants, so argument
//! serialization, transport failure, and response decoding are handled once.
//! Command names come from [`rusty_ipc`], which the backend uses too — they
//! cannot drift apart.
//!
//! Views never call these directly. Controllers do. A view that reaches for IPC
//! is a view that will eventually need a spinner, an error state, and a cancel
//! path, and those belong together in one place.

use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;

pub use rusty_ipc as cmd;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    /// Subscribe to a backend event. Returns an unlisten handle.
    ///
    /// For global signals only — device plugged in, catalogue reloaded. Streams
    /// that belong to one request (assistant answers, flash logs) use a
    /// [`Channel`] instead, so they cannot be crossed with another panel's.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], catch)]
    pub async fn listen(event: &str, handler: JsValue) -> Result<JsValue, JsValue>;

    /// `new window.__TAURI__.core.Channel()` — a per-request stream.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = Channel)]
    pub type Channel;

    #[wasm_bindgen(constructor, js_class = "Channel", js_namespace = ["window", "__TAURI__", "core"])]
    pub fn new() -> Channel;

    #[wasm_bindgen(method, setter, js_name = onmessage)]
    pub fn set_onmessage(this: &Channel, handler: &Closure<dyn FnMut(JsValue)>);

    /// The OS folder picker, from `tauri-plugin-dialog`.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "dialog"], catch, js_name = open)]
    async fn dialog_open(options: JsValue) -> Result<JsValue, JsValue>;
}

/// Ask the OS for a project folder.
///
/// `None` when the user cancels — a normal outcome, not an error, and one the
/// caller must not report as a failure.
pub async fn pick_folder(title: &str) -> Answer<Option<String>> {
    let options = serde_wasm_bindgen::to_value(&serde_json::json!({
        "directory": true,
        "multiple": false,
        "title": title,
    }))
    .map_err(|e| IpcError::local(format!("could not encode dialog options: {e}")))?;

    let selected = dialog_open(options)
        .await
        .map_err(|e| IpcError::from_js(&e))?;
    Ok(selected.as_string())
}

/// A failed command, as the backend describes it.
///
/// The cause chain is kept separate from the headline because for a broken
/// manifest the headline is generic and cargo's own diagnostic — the part that
/// says what to fix — is two levels down.
#[derive(Debug, Clone, Default)]
pub struct IpcError {
    pub message: String,
    pub causes: Vec<String>,
}

impl IpcError {
    fn from_js(value: &JsValue) -> Self {
        // The backend returns a serialised `CommandError`; anything else is a
        // transport failure, which arrives as a bare string.
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            message: String,
            #[serde(default)]
            causes: Vec<String>,
        }

        if let Ok(wire) = serde_wasm_bindgen::from_value::<Wire>(value.clone()) {
            return IpcError {
                message: wire.message,
                causes: wire.causes,
            };
        }
        IpcError {
            message: value
                .as_string()
                .unwrap_or_else(|| format!("{value:?}")),
            causes: Vec::new(),
        }
    }

    fn local(message: impl Into<String>) -> Self {
        IpcError {
            message: message.into(),
            causes: Vec::new(),
        }
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub type Answer<T> = Result<T, IpcError>;

/// Whether the Tauri bridge is present.
///
/// False when the frontend is served by `trunk serve` and opened in an ordinary
/// browser — a far faster loop for building panels than waiting on a full
/// `cargo tauri dev` rebuild.
///
/// Worth checking rather than letting the first call fail: `catch` on these
/// externs only converts a *rejected promise* into `Err`. With `window.__TAURI__`
/// absent, the shim throws a synchronous `TypeError` instead, which kills the
/// whole spawned task — so no error is recorded, no banner appears, and every
/// later step in that task silently never runs.
pub fn backend_available() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    js_sys::Reflect::get(&window, &JsValue::from_str("__TAURI__"))
        .map(|bridge| !bridge.is_undefined() && !bridge.is_null())
        .unwrap_or(false)
}

/// Typed call: serialize arguments, invoke, deserialize the response.
pub async fn call<A, R>(command: &str, args: &A) -> Answer<R>
where
    A: Serialize + ?Sized,
    R: DeserializeOwned,
{
    let args = serde_wasm_bindgen::to_value(args)
        .map_err(|e| IpcError::local(format!("could not encode arguments: {e}")))?;
    let value = invoke(command, args)
        .await
        .map_err(|e| IpcError::from_js(&e))?;
    serde_wasm_bindgen::from_value(value)
        .map_err(|e| IpcError::local(format!("could not decode the response: {e}")))
}

/// Typed call with no arguments.
pub async fn get<R: DeserializeOwned>(command: &str) -> Answer<R> {
    let value = invoke(command, JsValue::NULL)
        .await
        .map_err(|e| IpcError::from_js(&e))?;
    serde_wasm_bindgen::from_value(value)
        .map_err(|e| IpcError::local(format!("could not decode the response: {e}")))
}

/// Call a command that streams its output back through a channel.
///
/// The channel has to be grafted on with `Reflect` rather than serialized with
/// the rest of the arguments: it is a live JS object that Tauri recognises by
/// identity, and `serde_wasm_bindgen` would flatten it into a plain object. The
/// backend would then see no channel, and the failure is a type error naming a
/// field the caller thought it had set.
///
/// `channel_key` is the command's parameter name in camelCase — Tauri converts
/// `on_line` to `onLine` on the way in.
pub async fn call_streaming<A, R>(
    command: &str,
    args: &A,
    channel_key: &str,
    channel: &Channel,
) -> Answer<R>
where
    A: Serialize + ?Sized,
    R: DeserializeOwned,
{
    let args = serde_wasm_bindgen::to_value(args)
        .map_err(|e| IpcError::local(format!("could not encode arguments: {e}")))?;
    js_sys::Reflect::set(&args, &JsValue::from_str(channel_key), channel.as_ref())
        .map_err(|e| IpcError::from_js(&e))?;

    let value = invoke(command, args)
        .await
        .map_err(|e| IpcError::from_js(&e))?;
    serde_wasm_bindgen::from_value(value)
        .map_err(|e| IpcError::local(format!("could not decode the response: {e}")))
}
