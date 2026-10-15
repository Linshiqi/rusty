//! A real terminal for the workbench.
//!
//! [`model`] is the rendered screen the window draws; what opens a
//! pseudo-terminal or spawns a shell is in the modules beside it.
//!
//! Its own crate rather than a module of `rusty-embed`, because a terminal is
//! not embedded domain logic — it is infrastructure that an embedded workbench
//! happens to need, and burying it next to the chip catalogue would make the
//! next reader look for chips in it.

pub mod model;

pub use model::*;

pub mod builtin;
mod error;
pub mod pty;

pub use error::{Error, Result};
pub use pty::{Terminal, Updates, default_shell};
