//! Tauri commands.
//!
//! This layer is deliberately thin: it locates what is open, calls into
//! `rusty-embed`, `rusty-core` or `rusty-ai`, and converts errors. No analysis
//! lives here.
//!
//! It also honours the boundary rule from `docs/extensibility.md` — nothing
//! crossing into the WebView is anything other than a `model` type. No guppy
//! handles, no `Workspace`, no API keys.
//!
//! And nothing here blocks an async worker. Every filesystem walk, process
//! spawn, keychain read and `workbench.toml` write goes through
//! [`blocking`](crate::state::blocking):
//! the async workers are shared by every command in flight, and a `probe-rs
//! list` waiting on USB enumeration used to freeze the whole window for as
//! long as it took.

mod assistant;
mod crates;
mod devices;
mod disk;
mod embedded;
mod math;
mod project;
mod wizard;
mod workbench;

// Glob re-exports: `#[tauri::command]` puts a hidden macro beside each
// command, and `generate_handler!` finds it by the command's own path, so
// `commands::open_project` has to name both.
pub use assistant::*;
pub use crates::*;
pub use devices::*;
pub use disk::*;
pub use embedded::*;
pub use math::*;
pub use project::*;
pub use wizard::*;
pub use workbench::*;

use crate::error::CommandError;

type Answer<T> = Result<T, CommandError>;
