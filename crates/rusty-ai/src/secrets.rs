//! API keys live in the OS credential store, never on disk in plaintext.
//!
//! Windows Credential Manager, macOS Keychain, or the Linux Secret Service,
//! depending on the platform. Two rules follow from bring-your-own-key:
//!
//! 1. Keys are never written to a config file, so a user can share their rusty
//!    settings without leaking credentials.
//! 2. Keys are never sent to the WebView. Every LLM request is issued from the
//!    Rust side, so a compromised or misbehaving frontend cannot read them.

use keyring::Entry;

use crate::error::Result;

/// Namespace under which entries are filed in the OS store.
const SERVICE: &str = "dev.rusty.workbench";

fn entry(profile: &str) -> Result<Entry> {
    Ok(Entry::new(SERVICE, profile)?)
}

/// Store (or replace) the key for a provider profile.
pub fn store(profile: &str, api_key: &str) -> Result<()> {
    entry(profile)?.set_password(api_key)?;
    Ok(())
}

/// Look up a stored key. `Ok(None)` means the user has not configured this
/// profile yet, which is a normal state, not an error.
pub fn load(profile: &str) -> Result<Option<String>> {
    match entry(profile)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Forget a profile's key. Deleting one that was never stored is a no-op.
pub fn delete(profile: &str) -> Result<()> {
    match entry(profile)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Whether a key exists, without reading it.
///
/// The settings UI needs to show "configured" without ever pulling the secret
/// across a boundary it does not need to cross.
pub fn is_configured(profile: &str) -> bool {
    matches!(load(profile), Ok(Some(_)))
}
