//! What `workbench.toml` holds — the settings, the recent projects, each
//! project's tab strip, the assistant's profile — and the data directory it
//! lives in. Plus the one thing rusty hands to the desktop: a link.

use std::path::Path;

// The storage layer goes by its own name so it cannot be confused with
// rusty_ai's `config` at a call site.
use rusty_embed::config as storage;
use tauri::State;

use super::Answer;
use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

/// A setting typed into a text field: trimmed, and empty means unset.
fn typed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The proxy setting and what detection currently sees, for the settings page.
///
/// Detection reads the registry on Windows — a process spawn, not a lookup.
#[tauri::command]
pub async fn proxy_setting() -> Answer<serde_json::Value> {
    blocking("reading the proxy setting", || {
        let stored = storage::workbench().proxy;
        let detected = rusty_embed::net::system_proxy();
        serde_json::json!({ "stored": stored, "detected": detected })
    })
    .await
}

/// The stored shortcut overrides, id → chord.
#[tauri::command]
pub async fn keybinds() -> Answer<std::collections::BTreeMap<String, String>> {
    blocking("reading the shortcuts", || storage::workbench().keybinds).await
}

/// Whether modal editing is on. Read at startup by every window.
#[tauri::command]
pub async fn vim_enabled() -> Answer<bool> {
    blocking("reading the editor mode", || storage::workbench().vim).await
}

/// Turn modal editing on or off, for good and for every window.
#[tauri::command]
pub async fn set_vim(enabled: bool, state: State<'_, AppState>) -> Answer<()> {
    state
        .update_workbench(move |workbench| workbench.vim = enabled)
        .await
}

/// The rust-analyzer the user named, if any — empty means "whichever rusty
/// finds". Read at startup by every window, like the editor mode.
#[tauri::command]
pub async fn rust_analyzer_path() -> Answer<Option<String>> {
    blocking("reading the rust-analyzer setting", || {
        storage::workbench().rust_analyzer
    })
    .await
}

/// Name a rust-analyzer, or clear the choice. Takes effect on the next
/// language-server start, which is the next project open or window.
#[tauri::command]
pub async fn set_rust_analyzer_path(
    path: Option<String>,
    state: State<'_, AppState>,
) -> Answer<()> {
    let chosen = typed(path);
    state
        .update_workbench(move |workbench| workbench.rust_analyzer = chosen)
        .await
}

/// Whether the editor writes a beat after typing stops. Read at startup by
/// every window, like `vim_enabled`: a second window that did not auto-save
/// would lose work on the assumption that it had.
#[tauri::command]
pub async fn auto_save_enabled() -> Answer<bool> {
    blocking("reading the auto-save setting", || {
        storage::workbench().auto_save
    })
    .await
}

/// Turn auto-save on or off, for good and for every window.
#[tauri::command]
pub async fn set_auto_save(enabled: bool, state: State<'_, AppState>) -> Answer<()> {
    state
        .update_workbench(move |workbench| workbench.auto_save = enabled)
        .await
}

/// What the editor draws around the code. Read at startup by every window,
/// like `vim_enabled`, so two windows draw the editor alike.
#[tauri::command]
pub async fn editor_view() -> Answer<rusty_embed::EditorView> {
    blocking("reading the editor's settings", || {
        storage::workbench().editor
    })
    .await
}

/// Change what the editor draws, for good and for every window.
#[tauri::command]
pub async fn set_editor_view(
    view: rusty_embed::EditorView,
    state: State<'_, AppState>,
) -> Answer<()> {
    state
        .update_workbench(move |workbench| workbench.editor = view)
        .await
}

/// The stored display language, or `None` for "follow the system".
///
/// Read at startup by every window, like `vim_enabled`: two windows in two
/// languages is not a shrug.
#[tauri::command]
pub async fn display_locale() -> Answer<Option<String>> {
    blocking("reading the display language", || {
        storage::workbench().locale
    })
    .await
}

/// Choose the display language, for good and for every window.
#[tauri::command]
pub async fn set_display_locale(tag: Option<String>, state: State<'_, AppState>) -> Answer<()> {
    let locale = typed(tag);
    state
        .update_workbench(move |workbench| workbench.locale = locale)
        .await
}

/// Override one shortcut, or clear the override (chord = null) so the
/// built-in default applies again.
#[tauri::command]
pub async fn set_keybind(
    id: String,
    chord: Option<String>,
    state: State<'_, AppState>,
) -> Answer<()> {
    let chord = typed(chord);
    state
        .update_workbench(move |workbench| match chord {
            None => {
                workbench.keybinds.remove(&id);
            }
            Some(chord) => {
                workbench.keybinds.insert(id, chord);
            }
        })
        .await
}

/// Store the proxy choice: null/"auto" = detect, "none" = direct, else a URL.
#[tauri::command]
pub async fn set_proxy_setting(value: Option<String>, state: State<'_, AppState>) -> Answer<()> {
    let proxy = typed(value).filter(|value| value != "auto");
    state
        .update_workbench(move |workbench| workbench.proxy = proxy)
        .await
}

/// Projects opened before, newest first — what launch reopens and File lists.
#[tauri::command]
pub async fn recent_projects() -> Answer<Vec<String>> {
    blocking("reading the recent projects", || {
        storage::workbench().recent_projects
    })
    .await
}

/// Drop a recent that no longer exists. Called when reopening one fails, so a
/// moved project stops being offered every launch.
#[tauri::command]
pub async fn forget_recent(path: String, state: State<'_, AppState>) -> Answer<()> {
    state
        .with_workbench("forgetting a recent project", move || {
            storage::forget_recent(&path)
        })
        .await
}

/// Where rusty keeps its data, for the settings screen to show — the answer
/// to "what is this folder and may I delete it".
#[tauri::command]
pub async fn storage_location() -> Answer<Option<rusty_embed::StorageLocation>> {
    blocking("reading the data directory's location", storage::location).await
}

/// How much disk the data directory is using. Separate from `storage_location`
/// because it walks the tree, and most callers only want the path.
#[tauri::command]
pub async fn storage_footprint() -> Answer<u64> {
    blocking("measuring the data directory", storage::footprint).await
}

/// Move the data directory. Copies, switches the pointer, leaves the original
/// in place; with `take_existing` it adopts what the target already holds.
#[tauri::command]
pub async fn relocate_storage(
    path: String,
    take_existing: bool,
    state: State<'_, AppState>,
) -> Answer<rusty_embed::RelocateReport> {
    let report = blocking("relocation", move || {
        storage::relocate(Path::new(&path), take_existing)
    })
    .await??;
    // The cached catalogue was layered from the old directory.
    state.drop_catalog().await;
    Ok(report)
}

/// Remember a project's open editors.
///
/// Called on every tab switch, which is what made the lock necessary: this
/// read-modify-write landing between another writer's read and its save is
/// how a shortcut or a proxy setting used to vanish on the next launch.
#[tauri::command]
pub async fn record_tabs(
    root: String,
    tabs: Vec<String>,
    active: Option<String>,
    second: Vec<String>,
    second_active: Option<String>,
    state: State<'_, AppState>,
) -> Answer<()> {
    state
        .with_workbench("saving the tab strip", move || {
            storage::record_tabs(rusty_embed::ProjectTabs {
                root,
                tabs,
                active,
                second,
                second_active,
            })
        })
        .await
}

/// What a project had open last time.
#[tauri::command]
pub async fn project_tabs(root: String) -> Answer<Option<rusty_embed::ProjectTabs>> {
    blocking("reading the tab strip", move || {
        let mut strip = storage::tabs_for(&root)?;
        // Only the files that are still there. The strip is remembered per
        // project *directory*, and a directory can hold a different project
        // than it did last week — generating over a path somebody used
        // before is exactly what the wizard does. v0.6.30 made the restored
        // *active* file fail quietly; the rest sat on the strip as names,
        // and clicking one raised "could not read build.rs" about a file
        // from a layout that no longer exists.
        //
        // Answered here rather than in the frontend because it is one
        // `exists` per tab against a root the backend already has, where the
        // frontend would need a round trip each.
        let base = std::path::Path::new(&root);
        let here = |path: &String| base.join(path).exists();
        strip.tabs.retain(&here);
        strip.second.retain(&here);
        strip.active = strip.active.filter(|p| strip.tabs.contains(p));
        strip.second_active = strip.second_active.filter(|p| strip.second.contains(p));
        Some(strip)
    })
    .await
}

/// The assistant profile last chosen, and setting it.
///
/// Never the key: that lives in the OS credential store and is fetched by the
/// backend at the moment of the request, so it never enters the window.
#[tauri::command]
pub async fn assistant_choice() -> Answer<Option<rusty_embed::AssistantChoice>> {
    blocking("reading the assistant profile", || {
        storage::workbench().assistant
    })
    .await
}

/// Store the assistant profile. A save that fails says so — a profile the
/// user chose and the next launch does not remember is not a shrug.
#[tauri::command]
pub async fn set_assistant_choice(
    choice: rusty_embed::AssistantChoice,
    state: State<'_, AppState>,
) -> Answer<()> {
    state
        .update_workbench(move |workbench| workbench.assistant = Some(choice))
        .await
}

/// A link rusty may hand to the desktop, or why not.
///
/// Only https, and only RFC 3986's own alphabet with every `%` a complete
/// escape. Not because the openers need it — none of them goes through a
/// shell any more — but because a URL is the one string here that came from
/// outside (GitHub's `html_url`), and a rule about what it may contain is
/// cheaper than reasoning about what each opener does with a byte it did not
/// expect.
fn checked_url(url: &str) -> Result<&str, CommandError> {
    if !url.starts_with("https://") {
        return Err(CommandError::new("Only https links can be opened."));
    }
    let allowed =
        |byte: u8| byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&byte);
    if let Some(bad) = url.bytes().find(|byte| !allowed(*byte)) {
        return Err(CommandError::new(format!(
            "This link carries `{}`, which is not a character a URL can contain, so it was \
             not opened.",
            char::from(bad).escape_default(),
        )));
    }
    let bytes = url.as_bytes();
    let complete_escape = |at: usize| {
        bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit)
    };
    if let Some(at) = (0..bytes.len()).find(|&at| bytes[at] == b'%' && !complete_escape(at)) {
        return Err(CommandError::new(format!(
            "This link has a `%` at position {at} that is not a percent-encoding, so it was \
             not opened.",
        )));
    }
    Ok(url)
}

/// Hand a URL to the desktop. No plugin: the platform openers are stable and
/// this is the only thing rusty opens externally.
///
/// On Windows the opener is `rundll32 url.dll,FileProtocolHandler`, not `cmd
/// /C start`: `start` is a cmd built-in, so the URL went through cmd's parser,
/// and `&`, `|` and `^` in a query string were operators rather than
/// characters. The URL handler takes the string as one argument and passes it
/// on as one.
#[tauri::command]
pub async fn open_url(url: String) -> Answer<()> {
    let url = checked_url(&url)?.to_string();
    blocking("opening the link", move || {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("rundll32.exe", &["url.dll,FileProtocolHandler"])
        } else if cfg!(target_os = "macos") {
            ("open", &[])
        } else {
            ("xdg-open", &[])
        };
        std::process::Command::new(program)
            .args(args)
            .arg(&url)
            .spawn()
            .map(|_| ())
            .map_err(|e| CommandError::new(format!("could not open {url}: {e}")))
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The URL rule, pure: https only, RFC 3986's alphabet only, and every `%`
    /// a complete escape.
    #[test]
    fn only_a_well_formed_https_link_is_handed_to_the_desktop() {
        let release = "https://github.com/Linshiqi/rusty/releases/tag/v0.3.0";
        assert_eq!(checked_url(release).unwrap(), release);
        let query = "https://example.com/a?b=1&c=2#frag";
        assert_eq!(
            checked_url(query).unwrap(),
            query,
            "& and # are the URL's own characters — no shell is involved any more",
        );
        assert!(
            checked_url("https://example.com/a%20b").is_ok(),
            "a complete escape"
        );

        assert!(checked_url("http://example.com").is_err(), "https only");
        assert!(checked_url("file:///etc/passwd").is_err());
        assert!(
            checked_url("https://example.com/a b").is_err(),
            "a space is not URL"
        );
        assert!(
            checked_url("https://example.com/a|b").is_err(),
            "nor a pipe"
        );
        assert!(
            checked_url("https://example.com/a^b").is_err(),
            "nor cmd's escape"
        );
        assert!(
            checked_url("https://example.com/\"a\"").is_err(),
            "nor a quote"
        );
        assert!(
            checked_url("https://example.com/a<b>c").is_err(),
            "nor redirection"
        );
        assert!(
            checked_url("https://example.com/%PATH%").is_err(),
            "%PA is not an escape"
        );
        assert!(
            checked_url("https://example.com/a%2").is_err(),
            "a truncated escape"
        );
        assert!(
            checked_url("https://example.com/naïve").is_err(),
            "an IRI has to be encoded first"
        );
        assert!(
            checked_url("https://example.com/a\nb").is_err(),
            "no control characters"
        );
    }

    #[test]
    fn a_typed_setting_is_trimmed_and_blank_is_unset() {
        assert_eq!(typed(None), None);
        assert_eq!(typed(Some("".into())), None);
        assert_eq!(typed(Some("   ".into())), None);
        assert_eq!(typed(Some(" zh-CN ".into())), Some("zh-CN".to_string()));
    }
}
