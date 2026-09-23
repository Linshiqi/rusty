//! What this WebView keeps for itself in localStorage — the zoom factors,
//! the divider positions, the tree's fold, the diff layout — and nothing
//! else. `local_get`, `local_set` and `local_take` are the one door, so
//! the list in CLAUDE.md's "Where state lives" is a grep and not a claim.

use super::*;

/// This window's own storage — the short list the storage rule allows here:
/// theme, divider positions, the two zooms, the pin map's fold, the locale
/// cache. `None` when the WebView has nothing, which every caller reads as
/// "never chosen". One door, so the twelve copies of the
/// `window → local_storage → ok → flatten` chain became one.
pub fn local_get(key: &str) -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(key)
        .ok()
        .flatten()
}

pub fn local_set(key: &str, value: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, value);
    }
}

/// Read a key and delete it — for a value that moved to `workbench.toml`
/// and is carried across exactly once, so the key cannot linger to be
/// misread by a later version.
pub fn local_take(key: &str) -> Option<String> {
    let storage = web_sys::window()?.local_storage().ok().flatten()?;
    let raw = storage.get_item(key).ok().flatten()?;
    let _ = storage.remove_item(key);
    Some(raw)
}

/// The editor's font scale, as a factor of the 12.5px base. The wheel, the
/// settings buttons and the stored value all clamp to it, so a hand-edited
/// absurd value cannot produce a 300px caret. Three sites each spelled the
/// pair of numbers before this.
pub const EDITOR_ZOOM_RANGE: (f64, f64) = (0.6, 2.4);

/// The Markdown page's scale, the same span: a chapter read at arm's length
/// wants the same reach a listing does.
pub const PAGE_ZOOM_RANGE: (f64, f64) = (0.6, 2.4);

/// The interface scale — the slider's own range.
pub const UI_ZOOM_RANGE: (f64, f64) = (0.7, 1.6);

pub(super) fn stored_size(divider: Divider, fallback: f64) -> f64 {
    let (min, max) = divider.bounds();
    local_get(divider.storage_key())
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(fallback)
}

/// Editor font scale from last time.
pub(super) fn stored_zoom() -> f64 {
    local_get("rusty.editor.zoom")
        .and_then(|v| v.parse::<f64>().ok())
        .map(|z| z.clamp(EDITOR_ZOOM_RANGE.0, EDITOR_ZOOM_RANGE.1))
        .unwrap_or(1.0)
}

/// The page's scale from last time.
pub(super) fn stored_page_zoom() -> f64 {
    local_get("rusty.page.zoom")
        .and_then(|v| v.parse::<f64>().ok())
        .map(|z| z.clamp(PAGE_ZOOM_RANGE.0, PAGE_ZOOM_RANGE.1))
        .unwrap_or(1.0)
}

pub fn remember_page_zoom(zoom: f64) {
    local_set("rusty.page.zoom", &format!("{zoom:.2}"));
}

/// The interface scale from last time.
pub(super) fn stored_ui_zoom() -> f64 {
    local_get("rusty.ui.zoom")
        .and_then(|v| v.parse::<f64>().ok())
        .map(|z| z.clamp(UI_ZOOM_RANGE.0, UI_ZOOM_RANGE.1))
        .unwrap_or(1.0)
}

pub fn remember_ui_zoom(zoom: f64) {
    local_set("rusty.ui.zoom", &format!("{zoom:.2}"));
}

pub fn remember_zoom(zoom: f64) {
    local_set("rusty.editor.zoom", &format!("{zoom:.2}"));
}

pub fn remember_size(divider: Divider, value: f64) {
    local_set(divider.storage_key(), &value.to_string());
}

const TREE_HIDDEN_KEY: &str = "rusty.layout.tree-hidden";

/// Whether the file tree was folded away last time.
pub(super) fn stored_tree_hidden() -> bool {
    local_get(TREE_HIDDEN_KEY).is_some_and(|v| v == "hidden")
}

pub fn remember_tree_hidden(hidden: bool) {
    local_set(TREE_HIDDEN_KEY, if hidden { "hidden" } else { "shown" });
}

const SPLIT_KEY: &str = "rusty.git.split";

/// Side by side unless this window was told otherwise.
pub(super) fn stored_split() -> bool {
    local_get(SPLIT_KEY).is_none_or(|v| v != "unified")
}

pub fn remember_split(on: bool) {
    local_set(SPLIT_KEY, if on { "split" } else { "unified" });
}
