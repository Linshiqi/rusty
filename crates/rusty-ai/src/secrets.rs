//! API keys live in the OS credential store, never on disk in plaintext.
//!
//! Windows Credential Manager, macOS Keychain, or the Linux Secret Service,
//! depending on the platform. Two rules follow from bring-your-own-key:
//!
//! 1. Keys are never written to a config file, so a user can share their rusty
//!    settings without leaking credentials.
//! 2. Keys are never sent to the WebView. Every LLM request is issued from the
//!    Rust side, so a compromised or misbehaving frontend cannot read them.
//!
//! The store is behind a trait for one reason: a machine with no store. CI's
//! Linux runner has no Secret Service, and a test that could only talk to the
//! real keychain failed there for as long as nobody looked at the badge.
//! [`Keychain`] is the OS store and the only implementation the app uses;
//! [`Memory`] holds keys for the life of a process and exists for tests.

use std::{collections::BTreeMap, sync::Mutex};

use keyring::Entry;

use crate::error::Result;

/// Namespace under which entries are filed in the OS store.
const SERVICE: &str = "dev.rusty.workbench";

/// Where API keys are kept, by profile name.
pub trait Secrets: Send + Sync {
    /// Store (or replace) the key for a provider profile.
    fn store(&self, profile: &str, api_key: &str) -> Result<()>;

    /// Look up a stored key. `Ok(None)` means the user has not configured
    /// this profile yet, which is a normal state, not an error.
    fn load(&self, profile: &str) -> Result<Option<String>>;

    /// Forget a profile's key. Deleting one that was never stored is a no-op.
    fn delete(&self, profile: &str) -> Result<()>;

    /// Whether a key exists.
    ///
    /// This *reads* the entry — none of the stores offers a cheaper existence
    /// check — and drops it here; what the settings screen gets is the
    /// boolean. The promise is about the boundary the secret does not cross,
    /// not about the syscall.
    fn is_configured(&self, profile: &str) -> bool {
        matches!(self.load(profile), Ok(Some(_)))
    }
}

/// The OS credential store.
pub struct Keychain;

fn entry(profile: &str) -> Result<Entry> {
    Ok(Entry::new(SERVICE, profile)?)
}

impl Secrets for Keychain {
    fn store(&self, profile: &str, api_key: &str) -> Result<()> {
        entry(profile)?.set_password(api_key)?;
        Ok(())
    }

    fn load(&self, profile: &str) -> Result<Option<String>> {
        match entry(profile)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn delete(&self, profile: &str) -> Result<()> {
        match entry(profile)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Keys held in memory for the life of the process.
///
/// For tests, and for nothing else: a key stored here is gone at exit, which
/// is precisely the failure the keychain exists to prevent.
#[derive(Default)]
pub struct Memory {
    keys: Mutex<BTreeMap<String, String>>,
}

impl Secrets for Memory {
    fn store(&self, profile: &str, api_key: &str) -> Result<()> {
        self.keys
            .lock()
            .expect("secrets lock")
            .insert(profile.to_string(), api_key.to_string());
        Ok(())
    }

    fn load(&self, profile: &str) -> Result<Option<String>> {
        Ok(self
            .keys
            .lock()
            .expect("secrets lock")
            .get(profile)
            .cloned())
    }

    fn delete(&self, profile: &str) -> Result<()> {
        self.keys.lock().expect("secrets lock").remove(profile);
        Ok(())
    }
}

// The free functions are the keychain, for the callers that have no reason to
// choose — which is every caller in the app.

/// Store (or replace) the key for a provider profile, in the OS store.
pub fn store(profile: &str, api_key: &str) -> Result<()> {
    Keychain.store(profile, api_key)
}

/// A stored key from the OS store; `Ok(None)` when the profile has none.
pub fn load(profile: &str) -> Result<Option<String>> {
    Keychain.load(profile)
}

/// Forget a profile's key in the OS store.
pub fn delete(profile: &str) -> Result<()> {
    Keychain.delete(profile)
}

/// Whether the OS store holds a key for the profile. See
/// [`Secrets::is_configured`] for what that costs.
pub fn is_configured(profile: &str) -> bool {
    Keychain.is_configured(profile)
}
