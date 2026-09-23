//! The assistant's configuration: the presets, the tools, the key in the
//! credential store, and a provider checked or asked for its models. Asking
//! it something is `crate::ai`'s.

use rusty_ai::{Preset, ProviderCheck, ProviderConfig, ToolDef, ToolRegistry, secrets};

use super::Answer;
use crate::state::blocking;

#[tauri::command]
pub fn ai_presets() -> Vec<Preset> {
    rusty_ai::presets()
}

/// The tools the assistant can call, so the UI can show what it is allowed to
/// do before the user asks anything.
#[tauri::command]
pub fn ai_tools() -> Vec<ToolDef> {
    ToolRegistry::workbench().defs()
}

/// Whether a profile has a key on file. Deliberately returns a boolean and
/// never the key itself — the settings screen has no reason to hold a secret.
#[tauri::command]
pub async fn ai_key_configured(profile: String) -> Answer<bool> {
    blocking("reading the credential store", move || {
        secrets::is_configured(&profile)
    })
    .await
}

#[tauri::command]
pub async fn ai_store_key(profile: String, api_key: String) -> Answer<()> {
    Ok(blocking("writing the credential store", move || {
        secrets::store(&profile, &api_key)
    })
    .await??)
}

#[tauri::command]
pub async fn ai_delete_key(profile: String) -> Answer<()> {
    Ok(blocking("writing the credential store", move || {
        secrets::delete(&profile)
    })
    .await??)
}

/// The two things every provider call needs from the machine: the profile's
/// key and the proxy. One blocking hop for both — the keychain is IO, and the
/// proxy setting is a registry query on Windows.
///
/// The proxy is the same `effective_proxy` the tool installer and the update
/// check use, so the assistant reaches its endpoint on exactly the machines
/// where they reach theirs.
pub(crate) async fn ai_inputs(profile: String) -> Answer<(Option<String>, rusty_ai::Http)> {
    Ok(blocking("reading the assistant's key and proxy", move || {
        let key = secrets::load(&profile)?;
        let http = rusty_ai::Http {
            proxy: rusty_embed::net::effective_proxy(),
        };
        Ok::<_, rusty_ai::Error>((key, http))
    })
    .await??)
}

/// Ask the endpoint which models it serves.
///
/// Model names drift far too fast to ship as a hardcoded list, and a
/// self-hosted server's names are unknowable in advance.
#[tauri::command]
pub async fn ai_list_models(config: ProviderConfig) -> Answer<Vec<String>> {
    let (key, http) = ai_inputs(config.profile.clone()).await?;
    Ok(rusty_ai::config::list_models(&config, key, &http).await?)
}

/// Verify a provider profile end to end without starting a conversation.
///
/// What comes back is what one real request established, as facts the
/// frontend words — never a sentence, and never a success inferred from a
/// request that failed: a refused key, an unreachable host and a timeout are
/// the errors they are.
#[tauri::command]
pub async fn ai_check_provider(config: ProviderConfig) -> Answer<ProviderCheck> {
    let (key, http) = ai_inputs(config.profile.clone()).await?;
    Ok(rusty_ai::config::check(&config, key, &http).await?)
}
