use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use rusty_core::Workspace;
use rusty_embed::catalog::Catalog;

use crate::error::{Error, Result};

/// What a tool is allowed to look at.
///
/// Deliberately a bag of optionals rather than a required `Workspace`: the
/// assistant is useful before a project is fully loaded, and a tool that cannot
/// run should say *what is missing* rather than be unreachable. A user asking
/// "which ESP32 should I use for BLE?" has no project open at all, and the chip
/// catalogue can answer them anyway.
pub struct ToolContext<'a> {
    /// The resolved Cargo workspace, once `cargo metadata` has succeeded.
    pub workspace: Option<&'a Workspace>,
    /// Root directory of the open project.
    pub root: Option<&'a Path>,
    /// The most recently built firmware ELF, when one is known.
    pub firmware: Option<PathBuf>,
    /// Chips and boards after the user's and project's overlays.
    ///
    /// Passed in rather than looked up so the assistant answers from the same
    /// catalogue the panels show. A model that cannot see the board a user
    /// added would tell them it does not exist.
    pub catalog: Option<&'a Catalog>,
}

impl<'a> ToolContext<'a> {
    pub fn empty() -> Self {
        Self {
            workspace: None,
            root: None,
            firmware: None,
            catalog: None,
        }
    }

    pub fn with_workspace(workspace: &'a Workspace) -> Self {
        Self {
            workspace: Some(workspace),
            root: Some(workspace.root().as_std_path()),
            firmware: None,
            catalog: None,
        }
    }

    /// The catalogue in play, falling back to the built-ins.
    ///
    /// Borrowed when one was supplied, owned when it has to be built — so the
    /// common path costs nothing and the fallback is still correct rather than
    /// absent.
    pub fn catalog(&self) -> Cow<'_, Catalog> {
        match self.catalog {
            Some(catalog) => Cow::Borrowed(catalog),
            None => Cow::Owned(Catalog::builtin()),
        }
    }

    pub fn with_firmware(mut self, firmware: impl Into<PathBuf>) -> Self {
        self.firmware = Some(firmware.into());
        self
    }

    /// The workspace, or an error the model can act on.
    ///
    /// The message is addressed to the model rather than the user, because it
    /// goes back as a tool result: telling it *what to ask for* is what stops
    /// it inventing an answer instead.
    pub fn require_workspace(&self) -> Result<&Workspace> {
        self.workspace.ok_or_else(|| Error::MissingContext {
            needed: "an open Cargo workspace".into(),
            hint: "Ask the user to open a project folder, then try again.".into(),
        })
    }

    pub fn require_root(&self) -> Result<&Path> {
        self.root.ok_or_else(|| Error::MissingContext {
            needed: "an open project".into(),
            hint: "Ask the user to open a project folder, then try again.".into(),
        })
    }

    pub fn require_firmware(&self) -> Result<&Path> {
        self.firmware
            .as_deref()
            .ok_or_else(|| Error::MissingContext {
                needed: "a built firmware ELF".into(),
                hint: "The project has not been built yet, or the build failed. \
                   Suggest building before asking about memory use."
                    .into(),
            })
    }
}
