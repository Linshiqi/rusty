//! Cargo workspace analysis for the rusty workbench.
//!
//! Split in two by the `backend` feature:
//!
//! - [`model`] is always compiled and contains nothing but serde types. The
//!   Leptos frontend depends on this crate with `default-features = false` and
//!   `use`s these types directly — there is no generated binding layer, so the
//!   wire contract cannot drift out of sync.
//! - Everything else needs a filesystem and a `cargo metadata` subprocess, and
//!   is only compiled for the backend.
//!
//! ```no_run
//! # #[cfg(feature = "backend")]
//! # fn demo() -> Result<(), rusty_core::Error> {
//! use rusty_core::Workspace;
//!
//! let workspace = Workspace::load(".")?;
//! let report = workspace.report()?;
//! println!("{} crates resolved", report.vitals.resolved_deps);
//! # Ok(())
//! # }
//! ```

pub mod model;
pub use model::*;

#[cfg(feature = "backend")]
mod duplicates;
#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
mod features;
#[cfg(feature = "backend")]
mod overview;
#[cfg(feature = "backend")]
mod workspace;

#[cfg(feature = "backend")]
pub use error::{Error, Result};
#[cfg(feature = "backend")]
pub use workspace::Workspace;
