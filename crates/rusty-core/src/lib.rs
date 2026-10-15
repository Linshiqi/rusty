//! Cargo workspace analysis for the rusty workbench.
//!
//! [`model`] is what the analysis answers with — serde types and nothing
//! else, read alike by the window, the CLI and the assistant's tools. The
//! rest is the analysis: a filesystem and a `cargo metadata` subprocess.
//!
//! ```no_run
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

pub mod disk;
mod duplicates;
mod error;
mod features;
mod overview;
pub mod registry;
mod workspace;

pub use error::{Error, Result};
pub use workspace::Workspace;
