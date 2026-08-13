//! A real terminal for the workbench.
//!
//! Split by the `backend` feature exactly as the other crates are: [`model`] is
//! the rendered screen the Leptos frontend draws and compiles to wasm32;
//! everything that opens a pseudo-terminal or spawns a shell sits behind
//! `backend`.
//!
//! Its own crate rather than a module of `rusty-embed`, because a terminal is
//! not embedded domain logic — it is infrastructure that an embedded workbench
//! happens to need, and burying it next to the chip catalogue would make the
//! next reader look for chips in it.

pub mod model;

pub use model::*;

#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
pub mod pty;

#[cfg(feature = "backend")]
pub use error::{Error, Result};
#[cfg(feature = "backend")]
pub use pty::{Terminal, Updates, default_shell};
