use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
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
    /// Where the workspace comes from when `workspace` is `None`: loaded by
    /// the first tool that needs it, and not before.
    ///
    /// It was a workspace or nothing once, and the host only had one when
    /// something else had loaded it — so the assistant's Cargo tools told a
    /// user with a project open to open a project, until the Crates panel
    /// had happened to be visited.
    pub workspace_on_demand: Option<&'a LazyWorkspace>,
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
            workspace_on_demand: None,
            root: None,
            firmware: None,
            catalog: None,
        }
    }

    pub fn with_workspace(workspace: &'a Workspace) -> Self {
        Self {
            workspace: Some(workspace),
            workspace_on_demand: None,
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
        if let Some(workspace) = self.workspace {
            return Ok(workspace);
        }
        match self.workspace_on_demand {
            Some(lazy) => lazy.get().map_err(|reason| Error::MissingContext {
                needed: "the project's Cargo workspace".into(),
                hint: format!("Loading it with `cargo metadata` failed: {reason}"),
            }),
            None => Err(Error::MissingContext {
                needed: "an open Cargo workspace".into(),
                hint: "Ask the user to open a project folder, then try again.".into(),
            }),
        }
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

/// A Cargo workspace loaded the first time a tool asks for one, then kept.
///
/// Resolving the graph is `cargo metadata`: a tenth of a second on a small
/// project, most of a second on a large workspace, and unbounded where no
/// lockfile exists yet. Only the Cargo tools read it, so a question about a
/// chip or a file must not wait for it — and the host that knows the project
/// is open cannot know which question is coming.
pub struct LazyWorkspace {
    root: PathBuf,
    loaded: OnceLock<std::result::Result<Arc<Workspace>, String>>,
}

impl LazyWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            loaded: OnceLock::new(),
        }
    }

    /// One the host already holds, so nothing is loaded again.
    pub fn holding(root: impl Into<PathBuf>, workspace: Arc<Workspace>) -> Self {
        let loaded = OnceLock::new();
        let _ = loaded.set(Ok(workspace));
        Self {
            root: root.into(),
            loaded,
        }
    }

    /// The workspace, loading it if nothing has yet — or why it cannot be.
    ///
    /// A failure is kept for the life of this value like a success is: the
    /// second tool of one answer would only fail the same way, slowly.
    pub fn get(&self) -> std::result::Result<&Workspace, String> {
        let loaded = self.loaded.get_or_init(|| {
            Workspace::load(&self.root)
                .map(Arc::new)
                .map_err(|e| e.to_string())
        });
        match loaded {
            Ok(workspace) => Ok(workspace),
            Err(reason) => Err(reason.clone()),
        }
    }

    /// What was loaded, if anything was, for a host that keeps it for the
    /// next question.
    pub fn loaded(&self) -> Option<Arc<Workspace>> {
        self.loaded.get()?.as_ref().ok().cloned()
    }
}
